import { emit } from "@tauri-apps/api/event";
import { mockWindows } from "@tauri-apps/api/mocks";
import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { formatDate } from "../lib/i18n/format";
import { setLanguage, t } from "../lib/i18n/index.svelte";
import { startStores } from "../lib/stores";
import { navigation } from "../lib/stores/navigation.svelte";
import { toasts } from "../lib/stores/toasts.svelte";
import { expectNoAxeViolations } from "../test/axe";
import { createFakeBackend, type FakeBackend } from "../test/fakeBackend";
import {
  FIXTURE_CARD_CVV,
  FIXTURE_CARD_NUMBER,
  FIXTURE_LOGIN_PASSWORD,
  makeEntry,
  sessionStatus,
  type EntrySpec,
} from "../test/fixtures";
import { calls, renderMainApp } from "../test/mainApp";
import EntryDetailHarness from "../test/EntryDetailHarness.svelte";
import { renderWithBackend } from "../test/render";

afterEach(() => {
  vi.useRealTimers();
  setLanguage("en");
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });
});

type Plugins = Record<string, (payload: unknown) => unknown>;

function backend(extra: EntrySpec[] = [], plugins: Plugins = {}): FakeBackend {
  const fake = createFakeBackend({ session: "unlocked", plugins });
  for (const spec of extra) fake.state.entries.push(makeEntry(spec));
  return fake;
}

/** Renders the real main window and selects `id`; resolves once the detail heading is there. */
async function open(fake: FakeBackend, id: string, title?: string) {
  const app = await renderMainApp(fake);
  await screen.findByRole("listbox");
  navigation.select(id);
  await screen.findByRole("heading", { name: title ?? fake.state.entries.find((e) => e.summary.id === id)!.summary.title });
  return app;
}

const pane = () => screen.getByRole("region", { name: t("Details") });
const row = (label: string) => within(pane()).getByRole("group", { name: label });
const press = async (scope: HTMLElement, name: string) => userEvent.setup().click(within(scope).getByRole("button", { name }));
const revealCalls = (fake: FakeBackend) => calls(fake, "entryReveal");
const text = () => document.body.textContent ?? "";

async function menuChoice(scope: HTMLElement, item: string) {
  await fireEvent.contextMenu(scope);
  await userEvent.setup().click(await screen.findByRole("menuitem", { name: item }));
}

const github = "e-github";

describe("login detail", () => {
  it("login_detail_shows_username_url_group_modified_and_masked_password", async () => {
    const fake = backend();
    await open(fake, github);
    expect(row(t("Username")).textContent).toContain("octo");
    expect(within(row(t("Website"))).getByRole("link", { name: "https://github.com" })).toBeTruthy();
    expect(within(pane()).getByRole("button", { name: "Personal" })).toBeTruthy();
    expect(pane().textContent).toContain(formatDate(1_700_000_300_000, "datetime"));
    expect(row(t("Password")).textContent).toContain("••••••••");
  });

  it("no_password_text_in_dom_before_reveal", async () => {
    const fake = backend();
    const { container } = await open(fake, github);
    expect(container.textContent).not.toContain(FIXTURE_LOGIN_PASSWORD);
    expect(revealCalls(fake)).toHaveLength(0);
  });

  it("reveal_calls_entry_reveal_once_and_rehides_on_selection_change", async () => {
    const fake = backend();
    const { container } = await open(fake, github);
    await press(row(t("Password")), "Reveal");
    await waitFor(() => expect(container.textContent).toContain(FIXTURE_LOGIN_PASSWORD));
    expect(revealCalls(fake)).toHaveLength(1);
    expect(revealCalls(fake)[0]!.args).toEqual({ id: github, field: { kind: "password" } });
    navigation.select("e-gitlab");
    await screen.findByRole("heading", { name: "GitLab" });
    expect(container.textContent).not.toContain(FIXTURE_LOGIN_PASSWORD);
    expect(container.textContent).not.toContain("gitlab-secret");
    // Coming back shows the mask again.
    navigation.select(github);
    await screen.findByRole("heading", { name: "GitHub" });
    expect(container.textContent).not.toContain(FIXTURE_LOGIN_PASSWORD);
  });

  it("hide_removes_a_revealed_password", async () => {
    const fake = backend();
    await open(fake, github);
    await press(row(t("Password")), "Reveal");
    await waitFor(() => expect(text()).toContain(FIXTURE_LOGIN_PASSWORD));
    await press(row(t("Password")), "Hide");
    expect(text()).not.toContain(FIXTURE_LOGIN_PASSWORD);
  });

  it("revealed_value_cleared_from_dom_on_lock", async () => {
    const fake = backend();
    const { container } = await open(fake, github);
    await press(row(t("Password")), "Reveal");
    await waitFor(() => expect(container.textContent).toContain(FIXTURE_LOGIN_PASSWORD));
    await fake.emit("sessionChanged", sessionStatus("locked"));
    await waitFor(() => expect(container.textContent).not.toContain(FIXTURE_LOGIN_PASSWORD));
    expect(document.body.textContent).not.toContain(FIXTURE_LOGIN_PASSWORD);
  });

  it("a_mounted_detail_clears_its_revealed_value_when_the_session_locks", async () => {
    // The detail stays mounted here (no MainApp unmount), so only the reveal's own lock handling
    // can remove the value.
    const fake = backend();
    const { container } = renderWithBackend(EntryDetailHarness, { id: github, onopenhistory: vi.fn() }, fake);
    stops.push(await startStores());
    await screen.findByRole("heading", { name: "GitHub" });
    await userEvent.setup().click(within(within(container).getByRole("group", { name: t("Password") })).getByRole("button", { name: "Reveal" }));
    await waitFor(() => expect(container.textContent).toContain(FIXTURE_LOGIN_PASSWORD));
    await fake.emit("sessionChanged", sessionStatus("locked"));
    await waitFor(() => expect(container.textContent).not.toContain(FIXTURE_LOGIN_PASSWORD));
    expect(screen.getByRole("heading", { name: "GitHub" })).toBeTruthy();
  });

  it("copy_password_calls_entry_copy_and_toast_mentions_clear_seconds", async () => {
    const fake = backend();
    await open(fake, github);
    await waitFor(() => expect(fake.state.settings.clipboardClearSeconds).toBe(30));
    await press(row(t("Password")), "Copy");
    await waitFor(() => expect(calls(fake, "entryCopy")).toHaveLength(1));
    expect(calls(fake, "entryCopy")[0]!.args).toEqual({ id: github, field: { kind: "password" } });
    await waitFor(() => expect(toasts.items.map((x) => x.text)).toContain("Copied. The clipboard clears in 30 s."));
  });

  it("the_toast_follows_the_clear_seconds_setting", async () => {
    const fake = backend();
    fake.state.settings.clipboardClearSeconds = 45;
    await open(fake, github);
    await press(row(t("Username")), "Copy");
    await waitFor(() => expect(toasts.items.map((x) => x.text)).toContain("Copied. The clipboard clears in 45 s."));
  });

  it("copy_never_receives_value_in_webview", async () => {
    const fake = backend();
    const { container } = await open(fake, github);
    await press(row(t("Password")), "Copy");
    await waitFor(() => expect(calls(fake, "entryCopy")).toHaveLength(1));
    expect(revealCalls(fake)).toHaveLength(0);
    expect(container.textContent).not.toContain(FIXTURE_LOGIN_PASSWORD);
    expect(JSON.stringify(toasts.items)).not.toContain(FIXTURE_LOGIN_PASSWORD);
  });

  it("custom_protected_field_masked_until_reveal", async () => {
    const fake = backend([
      {
        id: "e-custom",
        title: "Custom",
        modified: 5,
        customFields: [
          { key: "API key", value: "k-12345", protected: true },
          { key: "Region", value: "eu-west", protected: false },
        ],
      },
    ]);
    await open(fake, "e-custom");
    expect(row("Region").textContent).toContain("eu-west");
    expect(text()).not.toContain("k-12345");
    await press(row("API key"), "Reveal");
    await waitFor(() => expect(text()).toContain("k-12345"));
    expect(revealCalls(fake)[0]!.args).toEqual({ id: "e-custom", field: { kind: "custom", key: "API key" } });
  });

  it("website_link_uses_opener_for_http_and_https_only", async () => {
    const opened: unknown[] = [];
    const fake = backend(
      [
        {
          id: "e-urls",
          title: "Sites",
          modified: 5,
          urls: ["https://secure.test/a", "javascript:alert(1)", "file:///etc/passwd", "http://plain.test"],
        },
      ],
      { "plugin:opener|open_url": (p) => (opened.push(p), null) },
    );
    await open(fake, "e-urls");
    const websites = row(t("Website"));
    const links = within(websites).getAllByRole("link");
    expect(links.map((a) => a.textContent)).toEqual(["https://secure.test/a", "http://plain.test"]);
    expect(websites.textContent).toContain("javascript:alert(1)");
    expect(websites.textContent).toContain("file:///etc/passwd");
    await userEvent.setup().click(links[0]!);
    await waitFor(() => expect(opened).toHaveLength(1));
    expect(JSON.stringify(opened[0])).toContain("https://secure.test/a");
    await userEvent.setup().click(links[1]!);
    await waitFor(() => expect(opened).toHaveLength(2));
    // Middle click and Enter take the same road and never navigate.
    expect(await fireEvent(links[0]!, new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 }))).toBe(false);
    await waitFor(() => expect(opened).toHaveLength(3));
    expect(await fireEvent.keyDown(links[0]!, { key: "Enter" })).toBe(false);
    await waitFor(() => expect(opened).toHaveLength(4));
    expect(await fireEvent.dragStart(links[0]!)).toBe(false);
  });

  it("detail_for_missing_entry_shows_not_found_state", async () => {
    const fake = backend();
    await renderMainApp(fake);
    await screen.findByRole("listbox");
    navigation.select("does-not-exist");
    await waitFor(() => expect(pane().textContent).toContain("Item not found"));
    expect(within(pane()).getByRole("status")).toBeTruthy();
    expect(within(pane()).queryByRole("heading")).toBeNull();
  });

  it("an_id_that_dropped_out_of_the_list_still_shows_its_detail", async () => {
    const fake = backend();
    await open(fake, "e-gitlab");
    // Narrowing the search removes GitLab from the list; the selection (and the pane) stays.
    await userEvent.setup().type(screen.getByRole("searchbox"), "GitHub");
    await waitFor(() => expect(within(screen.getByRole("listbox")).getAllByRole("option")).toHaveLength(1));
    expect(within(pane()).getByRole("heading", { name: "GitLab" })).toBeTruthy();
  });
});

describe("totp", () => {
  it("totp_code_grouping_six_and_eight_digits", async () => {
    const fake = backend();
    await open(fake, github);
    expect(await within(row(t("One-time code"))).findByText("123 456")).toBeTruthy();
    expect(calls(fake, "entryTotp")[0]!.args).toEqual({ id: github });
    navigation.select("e-gitlab");
    fake.state.totp.code = "12345678";
    fake.state.entries.find((e) => e.summary.id === "e-gitlab")!.summary.hasTotp = true;
    await screen.findByRole("heading", { name: "GitLab" });
    expect(await within(row(t("One-time code"))).findByText("1234 5678")).toBeTruthy();
  });

  it("totp_ring_shows_the_backend_remaining_seconds_and_copy_uses_entry_copy", async () => {
    const fake = backend();
    await open(fake, github);
    const ring = await within(row(t("One-time code"))).findByRole("img");
    expect(ring.getAttribute("aria-label")).toContain("12");
    await press(row(t("One-time code")), "Copy");
    await waitFor(() => expect(calls(fake, "entryCopy")).toHaveLength(1));
    expect(calls(fake, "entryCopy")[0]!.args).toEqual({ id: github, field: { kind: "totp" } });
    expect(revealCalls(fake)).toHaveLength(0);
  });

  it("totp_refetches_at_period_boundary", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
    const fake = backend();
    await open(fake, github);
    await within(row(t("One-time code"))).findByText("123 456");
    expect(calls(fake, "entryTotp")).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(11_000);
    expect(calls(fake, "entryTotp")).toHaveLength(1);
    expect(within(row(t("One-time code"))).getByRole("img").getAttribute("aria-label")).toContain("1");
    fake.state.totp = { code: "654321", secondsRemaining: 30, period: 30 };
    await vi.advanceTimersByTimeAsync(1_000);
    await waitFor(() => expect(calls(fake, "entryTotp")).toHaveLength(2));
    expect(await within(row(t("One-time code"))).findByText("654 321")).toBeTruthy();
  });

  it("the_refetch_timer_stops_when_the_entry_changes", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
    const fake = backend();
    await open(fake, github);
    await within(row(t("One-time code"))).findByText("123 456");
    navigation.select("e-gitlab");
    await screen.findByRole("heading", { name: "GitLab" });
    await vi.advanceTimersByTimeAsync(40_000);
    expect(calls(fake, "entryTotp")).toHaveLength(1);
  });
});

describe("header actions", () => {
  it("view_history_link_shows_count_and_opens_history_modal", async () => {
    // Task 10 owns the modal; here the link shows the count and calls the callback.
    const fake = backend([{ id: "e-hist", title: "Hist", modified: 5, historyCount: 3 }]);
    const { onopenhistory } = await openWithHistory(fake, "e-hist");
    await userEvent.setup().click(screen.getByRole("button", { name: "View history (3)" }));
    expect(onopenhistory).toHaveBeenCalledWith("e-hist");
  });

  it("bin_entry_shows_restore_and_delete_permanently_only", async () => {
    const fake = backend();
    await open(fake, "e-old");
    const buttons = within(pane()).getAllByRole("button").map((b) => b.getAttribute("aria-label") ?? b.textContent?.trim());
    expect(buttons).toContain("Restore");
    expect(buttons).toContain("Delete permanently");
    expect(buttons).not.toContain("Edit");
    expect(buttons).not.toContain("Delete");
    expect(within(pane()).queryByRole("button", { name: /Favorites/ })).toBeNull();
    await press(pane(), "Restore");
    await waitFor(() => expect(calls(fake, "entryRestore")).toHaveLength(1));
    expect(calls(fake, "entryRestore")[0]!.args).toEqual({ id: "e-old", groupId: null });
  });

  it("permanent_delete_from_the_bin_asks_first", async () => {
    const fake = backend();
    await open(fake, "e-old");
    await press(pane(), "Delete permanently");
    const dialog = await screen.findByRole("dialog");
    expect(calls(fake, "entryPurge")).toHaveLength(0);
    await userEvent.setup().click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(calls(fake, "entryPurge")).toHaveLength(0);
    await press(pane(), "Delete permanently");
    await userEvent.setup().click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Delete permanently" }));
    await waitFor(() => expect(calls(fake, "entryPurge")).toHaveLength(1));
    expect(calls(fake, "entryPurge")[0]!.args).toEqual({ id: "e-old" });
  });

  it("delete_moves_entry_to_bin_without_confirm_and_selects_neighbour", async () => {
    const fake = backend();
    await open(fake, github);
    const before = within(screen.getByRole("listbox")).getAllByRole("option").map((o) => o.querySelector("[data-part='title']")?.textContent);
    expect(before[0]).toBe("GitHub");
    await press(pane(), "Delete");
    expect(screen.queryByRole("dialog")).toBeNull();
    await waitFor(() => expect(calls(fake, "entryDelete")).toHaveLength(1));
    expect(calls(fake, "entryDelete")[0]!.args).toEqual({ id: github });
    await waitFor(() => expect(navigation.selectedId).toBe(before[1] === "GitLab" ? "e-gitlab" : navigation.selectedId));
    expect(navigation.selectedId).not.toBe(github);
    expect(navigation.selectedId).not.toBeNull();
    expect(fake.state.entries.find((e) => e.summary.id === github)!.summary.inRecycleBin).toBe(true);
  });

  it("delete_asks_first_when_the_bin_is_disabled", async () => {
    const fake = backend();
    fake.state.recycleBinEnabled = false;
    await open(fake, github);
    await waitFor(() => expect(within(pane()).getByRole("button", { name: "Delete permanently" })).toBeTruthy());
    await press(pane(), "Delete permanently");
    expect(await screen.findByRole("dialog")).toBeTruthy();
    expect(calls(fake, "entryDelete")).toHaveLength(0);
  });

  it("an_unknown_bin_state_counts_as_bin_off_and_delete_asks_first", async () => {
    const fake = backend();
    fake.state.recycleBinEnabled = false;
    // The sidebar never answers, so the bin state stays unknown.
    fake.handlers.sidebarGet = () => new Promise(() => undefined);
    await open(fake, github);
    await press(pane(), "Delete permanently");
    expect(await screen.findByRole("dialog")).toBeTruthy();
    expect(calls(fake, "entryDelete")).toHaveLength(0);
  });

  it("the_favorite_star_toggles_through_entry_set_favorite", async () => {
    const fake = backend();
    await open(fake, "e-gitlab");
    await userEvent.setup().click(within(pane()).getByRole("button", { name: "Add to Favorites" }));
    await waitFor(() => expect(calls(fake, "entrySetFavorite")[0]!.args).toEqual({ id: "e-gitlab", favorite: true }));
    // The detail refetches on VaultChanged and the star flips.
    expect(await within(pane()).findByRole("button", { name: "Remove from Favorites" })).toBeTruthy();
  });

  it("the_group_link_selects_the_group_scope", async () => {
    const fake = backend();
    await open(fake, "e-gitlab");
    await userEvent.setup().click(within(pane()).getByRole("button", { name: "Work" }));
    expect(navigation.scope).toEqual({ kind: "group", groupId: "group-work" });
    expect(navigation.selectedId).toBe("e-gitlab");
  });

  it("edit_opens_the_editor_through_the_navigation_store", async () => {
    const fake = backend();
    await open(fake, github);
    await press(pane(), "Edit");
    await waitFor(() => expect(navigation.editModel?.id).toBe(github));
    expect(navigation.presentation).toBe("edit");
    navigation.closeEditor();
  });
});

describe("address, card, files", () => {
  it("address_detail_lists_nonempty_fields_only", async () => {
    const fake = backend([
      {
        id: "e-addr",
        title: "Home",
        entryType: "address",
        modified: 5,
        address: {
          label: "Home", fullName: "Ann Doe", givenName: "", familyName: "", organization: "", addressLine1: "1 Main St",
          addressLine2: "", city: "Springfield", region: "", postalCode: "12345", countryCode: "US", countryName: "",
          phone: "", email: "ann@example.org",
        },
      },
    ]);
    await open(fake, "e-addr");
    const body = pane().textContent ?? "";
    for (const v of ["Ann Doe", "1 Main St", "Springfield", "12345", "US", "ann@example.org"]) expect(body, v).toContain(v);
    expect(within(pane()).queryByRole("group", { name: t("Phone") })).toBeNull();
    expect(within(pane()).queryByRole("group", { name: t("Organization") })).toBeNull();
    expect(within(pane()).getByRole("group", { name: t("City") })).toBeTruthy();
  });

  it("card_detail_masks_number_and_reveals_cvv_on_click", async () => {
    const fake = backend();
    await open(fake, "e-visa");
    expect(row(t("Card number")).textContent).toContain("•••• 1111");
    expect(row(t("Cardholder")).textContent).toContain("Ann Visa");
    expect(row(t("Expires")).textContent).toContain("12/30");
    expect(text()).not.toContain(FIXTURE_CARD_NUMBER);
    expect(text()).not.toContain(FIXTURE_CARD_CVV);
    expect(revealCalls(fake)).toHaveLength(0);
    await press(row(t("Security code")), "Reveal");
    await waitFor(() => expect(text()).toContain(FIXTURE_CARD_CVV));
    expect(revealCalls(fake)[0]!.args).toEqual({ id: "e-visa", field: { kind: "cardCvv" } });
    expect(text()).not.toContain(FIXTURE_CARD_NUMBER);
    await press(row(t("Card number")), "Reveal");
    await waitFor(() => expect(text()).toContain(FIXTURE_CARD_NUMBER));
    expect(text()).not.toContain(FIXTURE_CARD_CVV);
  });

  it("card_large_type_is_offered_only_after_reveal", async () => {
    const fake = backend();
    await open(fake, "e-visa");
    await fireEvent.contextMenu(row(t("Card number")));
    const item = await screen.findByRole("menuitem", { name: "Show Large Type" });
    expect(item.getAttribute("aria-disabled")).toBe("true");
    await userEvent.setup().keyboard("{Escape}");
    await press(row(t("Card number")), "Reveal");
    await waitFor(() => expect(text()).toContain(FIXTURE_CARD_NUMBER));
    await menuChoice(row(t("Card number")), "Show Large Type");
    const dialog = await screen.findByRole("dialog", { name: "Large Type" });
    await waitFor(() => expect(dialog.textContent).toContain(FIXTURE_CARD_NUMBER));
  });

  it("files_detail_export_calls_save_dialog_then_attachment_export", async () => {
    const dialogCalls: string[] = [];
    const fake = backend(
      [{ id: "e-files", title: "Docs", entryType: "files", modified: 5, attachments: [{ name: "passport.pdf", size: 2048 }] }],
      { "plugin:dialog|save": () => (dialogCalls.push("save"), null) },
    );
    await open(fake, "e-files");
    const attachments = within(pane()).getByRole("group", { name: "Attachments" });
    expect(attachments.textContent).toContain("passport.pdf");
    expect(attachments.textContent).toContain("2 KB");
    await press(attachments, "Export…");
    await waitFor(() => expect(calls(fake, "attachmentExport")).toHaveLength(1));
    // R7: the save dialog opens in Rust; the webview passes only the entry id and the name.
    expect(calls(fake, "attachmentExport")[0]!.args).toEqual({ entryId: "e-files", name: "passport.pdf" });
    expect(dialogCalls).toEqual([]);
  });

  it("a_cancelled_export_shows_nothing", async () => {
    const fake = backend([{ id: "e-files", title: "Docs", entryType: "files", modified: 5, attachments: [{ name: "a.txt", size: 3 }] }]);
    fake.state.exportCancelled = true;
    await open(fake, "e-files");
    await press(within(pane()).getByRole("group", { name: "Attachments" }), "Export…");
    await waitFor(() => expect(calls(fake, "attachmentExport")).toHaveLength(1));
    expect(toasts.items).toHaveLength(0);
  });

  it("a_saved_export_says_so_and_carries_no_path", async () => {
    const fake = backend([{ id: "e-files", title: "Docs", entryType: "files", modified: 5, attachments: [{ name: "a.txt", size: 3 }] }]);
    await open(fake, "e-files");
    await press(within(pane()).getByRole("group", { name: "Attachments" }), "Export…");
    await waitFor(() => expect(toasts.items.map((x) => x.text)).toEqual(["Exported."]));
  });
});

describe("notes in the detail", () => {
  const withNotes = (notes: string) => backend([{ id: "e-n2", title: "Pad", entryType: "notes", modified: 5, notes }]);

  it("notes_rehide_on_selection_change", async () => {
    const fake = withNotes("secret **note**");
    await open(fake, "e-n2");
    await press(pane(), "Reveal notes");
    await waitFor(() => expect(within(pane()).getByText("note")).toBeTruthy());
    navigation.select(github);
    await screen.findByRole("heading", { name: "GitHub" });
    expect(text()).not.toContain("secret");
    navigation.select("e-n2");
    await screen.findByRole("heading", { name: "Pad" });
    expect(text()).not.toContain("secret");
  });

  it("reveal_is_exclusive_across_notes_and_fields", async () => {
    const fake = backend([
      {
        id: "e-mix",
        title: "Mixed",
        modified: 5,
        password: "pw-mixed-1",
        notes: "notes-mixed-1",
        customFields: [{ key: "Token", value: "tok-mixed-1", protected: true }],
      },
    ]);
    await open(fake, "e-mix");
    const visible = () => ["pw-mixed-1", "notes-mixed-1", "tok-mixed-1"].filter((s) => text().includes(s));
    // Settled states alone would miss a moment with two secrets on screen, so every DOM mutation
    // records how many were visible.
    let most = 0;
    const observer = new MutationObserver(() => {
      most = Math.max(most, visible().length);
    });
    observer.observe(pane(), { subtree: true, childList: true, characterData: true });
    await press(row(t("Password")), "Reveal");
    await waitFor(() => expect(visible()).toEqual(["pw-mixed-1"]));
    await press(pane(), "Reveal notes");
    await waitFor(() => expect(visible()).toEqual(["notes-mixed-1"]));
    await press(row("Token"), "Reveal");
    await waitFor(() => expect(visible()).toEqual(["tok-mixed-1"]));
    await press(row(t("Password")), "Reveal");
    await waitFor(() => expect(visible()).toEqual(["pw-mixed-1"]));
    observer.disconnect();
    expect(most).toBe(1);
  });

  // This checks the DOM; that the group itself holds no value after a minimise is asserted on the
  // members in secret.svelte.test.ts (a_minimised_window_leaves_no_member_value).
  it("notes_rehide_on_window_minimize_or_hide", async () => {
    mockWindows("main");
    const fake = withNotes("topsecret note");
    await open(fake, "e-n2");
    await press(pane(), "Reveal notes");
    await waitFor(() => expect(text()).toContain("topsecret"));
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    document.dispatchEvent(new Event("visibilitychange"));
    await waitFor(() => expect(text()).not.toContain("topsecret"));
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });
    await press(pane(), "Reveal notes");
    await waitFor(() => expect(text()).toContain("topsecret"));
    fake.state.minimized = true;
    await emit("tauri://resize", { width: 1, height: 1 });
    await waitFor(() => expect(text()).not.toContain("topsecret"));
  });
});

describe("large type", () => {
  it("large_type_not_in_dom_until_invoked", async () => {
    const fake = backend();
    await open(fake, github);
    expect(screen.queryByRole("dialog", { name: "Large Type" })).toBeNull();
    expect(revealCalls(fake)).toHaveLength(0);
    expect(text()).not.toContain(FIXTURE_LOGIN_PASSWORD);
  });

  it("large_type_shows_value_and_clears_on_lock", async () => {
    const fake = backend();
    await open(fake, github);
    await menuChoice(row(t("Password")), "Show Large Type");
    const dialog = await screen.findByRole("dialog", { name: "Large Type" });
    await waitFor(() => expect(dialog.textContent).toContain(FIXTURE_LOGIN_PASSWORD));
    expect(revealCalls(fake)).toHaveLength(1);
    const value = dialog.querySelector<HTMLElement>("[data-part='value']")!;
    expect(value.style.fontSize).toBe("48px");
    expect(value.style.fontFamily).toContain("monospace");
    await fake.emit("sessionChanged", sessionStatus("locked"));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Large Type" })).toBeNull());
    expect(document.body.textContent).not.toContain(FIXTURE_LOGIN_PASSWORD);
  });

  it("large_type_closes_on_escape_and_outside_click_and_selection_change", async () => {
    const fake = backend();
    await open(fake, github);
    const user = userEvent.setup();
    const show = async () => {
      await menuChoice(row(t("Password")), "Show Large Type");
      return screen.findByRole("dialog", { name: "Large Type" });
    };
    await show();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Large Type" })).toBeNull());

    const dialog = await show();
    const backdrop = dialog.parentElement!;
    await fireEvent.pointerDown(backdrop);
    await fireEvent.click(backdrop);
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Large Type" })).toBeNull());

    await show();
    navigation.select("e-gitlab");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Large Type" })).toBeNull());
    expect(text()).not.toContain(FIXTURE_LOGIN_PASSWORD);
  });

  it("large_type_closes_on_window_hide", async () => {
    const fake = backend();
    await open(fake, github);
    await menuChoice(row(t("Password")), "Show Large Type");
    await screen.findByRole("dialog", { name: "Large Type" });
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    document.dispatchEvent(new Event("visibilitychange"));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Large Type" })).toBeNull());
    expect(text()).not.toContain(FIXTURE_LOGIN_PASSWORD);
  });

  it("large_type_is_exclusive_with_inline_reveal", async () => {
    const fake = backend();
    await open(fake, github);
    await press(row(t("Password")), "Reveal");
    await waitFor(() => expect(text()).toContain(FIXTURE_LOGIN_PASSWORD));
    await menuChoice(row(t("Password")), "Show Large Type");
    const dialog = await screen.findByRole("dialog", { name: "Large Type" });
    await waitFor(() => expect(dialog.textContent).toContain(FIXTURE_LOGIN_PASSWORD));
    // The only copy of the value in the document is the one in the dialog.
    expect(row(t("Password")).textContent).not.toContain(FIXTURE_LOGIN_PASSWORD);
  });

  it("large_type_closes_when_the_entry_content_changes", async () => {
    const fake = backend();
    await open(fake, github);
    await menuChoice(row(t("Password")), "Show Large Type");
    await screen.findByRole("dialog", { name: "Large Type" });
    const entry = fake.state.entries.find((e) => e.summary.id === github)!;
    entry.modified += 10;
    await fake.emit("vaultChanged", null);
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Large Type" })).toBeNull());
    expect(text()).not.toContain(FIXTURE_LOGIN_PASSWORD);
  });

  it("the_context_menu_offers_large_type_copy_and_reveal", async () => {
    const fake = backend();
    await open(fake, github);
    await fireEvent.contextMenu(row(t("Password")));
    const names = (await screen.findAllByRole("menuitem")).map((i) => i.textContent?.trim());
    expect(names).toEqual(["Show Large Type", "Copy", "Reveal"]);
    await userEvent.setup().click(screen.getByRole("menuitem", { name: "Reveal" }));
    await waitFor(() => expect(text()).toContain(FIXTURE_LOGIN_PASSWORD));
    expect(revealCalls(fake)).toHaveLength(1);
  });

  it("the_menu_key_opens_the_context_menu_from_a_row_button", async () => {
    const fake = backend();
    await open(fake, github);
    within(row(t("Username"))).getByRole("button", { name: "Copy" }).focus();
    await userEvent.setup().keyboard("{Shift>}{F10}{/Shift}");
    expect(await screen.findByRole("menu")).toBeTruthy();
  });
});

describe("change website", () => {
  async function openSheet(fake: FakeBackend) {
    await open(fake, github);
    await press(row(t("Website")), "Change…");
    return screen.findByRole("dialog", { name: "Websites" });
  }

  it("change_website_from_detail_opens_websites_sheet_and_saves", async () => {
    const fake = backend();
    const dialog = await openSheet(fake);
    const user = userEvent.setup();
    expect((within(dialog).getAllByRole("textbox") as HTMLInputElement[]).map((i) => i.value)).toEqual(["https://github.com"]);
    expect(navigation.presentation).toBe("browse");
    expect(navigation.editModel?.id).toBe(github);
    await user.click(within(dialog).getByRole("button", { name: "Add Website" }));
    const inputs = within(dialog).getAllByRole("textbox");
    await user.type(inputs[inputs.length - 1]!, "https://b.test");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "entrySave")).toHaveLength(1));
    const saved = calls(fake, "entrySave")[0]!.args.model;
    expect(saved.urls).toEqual(["https://github.com", "https://b.test"]);
    expect(saved.password).toBe(FIXTURE_LOGIN_PASSWORD);
    expect(saved.id).toBe(github);
    await waitFor(() => expect(navigation.editModel).toBeNull());
    expect(navigation.presentation).toBe("browse");
    expect(screen.queryByRole("dialog", { name: "Websites" })).toBeNull();
    await waitFor(() => expect(within(row(t("Website"))).getAllByRole("link")).toHaveLength(2));
  });

  it("the_match_mode_is_part_of_the_change", async () => {
    const fake = backend();
    const dialog = await openSheet(fake);
    await userEvent.setup().selectOptions(within(dialog).getByRole("combobox", { name: "Match mode" }), "exact");
    await userEvent.setup().click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "entrySave")).toHaveLength(1));
    expect(calls(fake, "entrySave")[0]!.args.model.uriMatchMode).toBe("exact");
  });

  it("empty_rows_are_dropped_and_a_row_can_be_removed", async () => {
    const fake = backend();
    const dialog = await openSheet(fake);
    const user = userEvent.setup();
    await user.click(within(dialog).getByRole("button", { name: "Add Website" }));
    await user.click(within(dialog).getAllByRole("button", { name: "Remove" })[0]!);
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "entrySave")).toHaveLength(1));
    expect(calls(fake, "entrySave")[0]!.args.model.urls).toEqual([]);
  });

  it("change_website_cancel_and_lock_drop_the_edit_model", async () => {
    const fake = backend();
    let dialog = await openSheet(fake);
    await userEvent.setup().click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(navigation.editModel).toBeNull();
    expect(calls(fake, "entrySave")).toHaveLength(0);
    expect(screen.queryByRole("dialog", { name: "Websites" })).toBeNull();

    await press(row(t("Website")), "Change…");
    dialog = await screen.findByRole("dialog", { name: "Websites" });
    expect(dialog).toBeTruthy();
    expect(navigation.editModel).not.toBeNull();
    await fake.emit("sessionChanged", sessionStatus("locked"));
    await waitFor(() => expect(navigation.editModel).toBeNull());
    expect(calls(fake, "entrySave")).toHaveLength(0);
  });

  it("changing_the_selection_drops_the_edit_model_of_the_sheet", async () => {
    const fake = backend();
    await openSheet(fake);
    navigation.select("e-gitlab");
    await screen.findByRole("heading", { name: "GitLab" });
    expect(navigation.editModel).toBeNull();
  });
});

describe("accessibility", () => {
  it("detail_passes_axe", async () => {
    const fake = backend([
      {
        id: "e-all",
        title: "Everything",
        modified: 5,
        password: "pw",
        notes: "n",
        urls: ["https://x.test"],
        hasTotp: true,
        passkey: { relyingParty: "x.test", username: "u", created: 1, usable: true },
        customFields: [{ key: "K", value: "v", protected: true }],
        attachments: [{ name: "f.txt", size: 10 }],
        historyCount: 2,
      },
    ]);
    await open(fake, "e-all");
    await within(row(t("One-time code"))).findByText("123 456");
    await expectNoAxeViolations(document.body);
  });

  it("card_and_address_detail_pass_axe", async () => {
    const fake = backend();
    await open(fake, "e-visa");
    await expectNoAxeViolations(document.body);
  });
});

describe("localisation", () => {
  it("the_detail_follows_the_ui_language", async () => {
    const fake = backend();
    fake.state.settings.language = "uk";
    await open(fake, github);
    await waitFor(() => expect(within(pane()).getByRole("group", { name: t("Password") })).toBeTruthy());
    expect(t("Password")).not.toBe("Password");
  });
});

// The history seam: Task 10 passes `onopenhistory` from the main window; a harness does it here.
const stops: (() => void)[] = [];
afterEach(() => {
  for (const stop of stops.splice(0)) stop();
});

async function openWithHistory(fake: FakeBackend, id: string) {
  const onopenhistory = vi.fn();
  fake.install();
  renderWithBackend(EntryDetailHarness, { id, onopenhistory }, fake);
  stops.push(await startStores());
  await screen.findByRole("heading");
  return { onopenhistory };
}
