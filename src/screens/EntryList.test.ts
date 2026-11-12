import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { ENTRY_MIME } from "../lib/dnd";
import { setLanguage, t } from "../lib/i18n/index.svelte";
import en from "../lib/i18n/locales/en.json";
import ru from "../lib/i18n/locales/ru.json";
import { navigation } from "../lib/stores/navigation.svelte";
import { toasts } from "../lib/stores/toasts.svelte";
import { FakeDataTransfer } from "../test/dnd";
import { expectNoAxeViolations } from "../test/axe";
import { createFakeBackend, type FakeBackend } from "../test/fakeBackend";
import {
  BIN_GROUP_ID,
  FIXTURE_LOGIN_PASSWORD,
  WORK_GROUP_ID,
  fixtureSidebar,
  makeEntry,
  type EntrySpec,
} from "../test/fixtures";
import { calls, renderMainApp } from "../test/mainApp";
import type { AddressDto, Category } from "../lib/bindings";

afterEach(() => setLanguage("en"));

type Plugins = Record<string, (payload: unknown) => unknown>;

function backend(extra: EntrySpec[] = [], plugins: Plugins = {}): FakeBackend {
  const fake = createFakeBackend({ session: "unlocked", plugins });
  for (const spec of extra) fake.state.entries.push(makeEntry(spec));
  return fake;
}

async function open(fake: FakeBackend = backend()) {
  const app = await renderMainApp(fake);
  await screen.findByRole("listbox");
  return app;
}

// The sort select has `option` elements of its own, so rows are looked up inside the listbox.
const options = () => {
  const box = screen.queryByRole("listbox");
  return box ? within(box).queryAllByRole("option") : [];
};
const optionNamed = (name: RegExp | string) => within(screen.getByRole("listbox")).getByRole("option", { name });
const titles = () => options().map((o) => o.querySelector("[data-part='title']")?.textContent ?? "");

const address = (city: string): AddressDto => ({
  label: "Home",
  fullName: "",
  givenName: "",
  familyName: "",
  organization: "",
  addressLine1: "",
  addressLine2: "",
  city,
  region: "",
  postalCode: "",
  countryCode: "",
  countryName: "",
  phone: "",
  email: "",
});

/** Types into the search box and waits until the store has asked the backend for exactly that text. */
async function search(fake: FakeBackend, text: string, clear = false) {
  const user = userEvent.setup();
  const box = screen.getByRole("searchbox");
  if (clear) await user.clear(box);
  if (text !== "") await user.type(box, text);
  await waitFor(() => expect((calls(fake, "entryList").at(-1)?.args.query as { search: string }).search).toBe(text));
}

describe("rows", () => {
  it("rows_show_letter_avatar_title_and_subtitle", async () => {
    await open();
    const github = optionNamed(/GitHub/);
    expect(within(github).getByText("G")).toBeTruthy();
    expect(within(github).getByText("GitHub")).toBeTruthy();
    expect(within(github).getByText("octo")).toBeTruthy();
  });

  it("badges_for_favorite_totp_passkey_and_attachments", async () => {
    await open(
      backend([
        {
          id: "e-pk",
          title: "Zed",
          hasPasskey: true,
          attachments: [
            { name: "a.txt", size: 1 },
            { name: "b.txt", size: 2 },
          ],
          modified: 1,
        },
      ]),
    );
    const github = optionNamed(/GitHub/);
    expect(within(github).getByRole("img", { name: t("Favorites") })).toBeTruthy();
    expect(within(github).getByRole("img", { name: t("Verification Code") })).toBeTruthy();
    expect(within(github).queryByRole("img", { name: t("Passkey") })).toBeNull();
    const zed = optionNamed(/Zed/);
    expect(within(zed).getByRole("img", { name: t("Passkey") })).toBeTruthy();
    expect(within(zed).getByRole("img", { name: t("%lld files", 2) })).toBeTruthy();
    expect(within(zed).queryByRole("img", { name: t("Favorites") })).toBeNull();
  });

  it("security_flags_rendered_as_localised_chips", async () => {
    const flagged = (flags: EntrySpec["flags"], id: string, title: string): EntrySpec => ({ id, title, flags, modified: 1 });
    await open(
      backend([
        flagged(["reused"], "e-r", "Reused one"),
        flagged(["emptyPassword"], "e-n", "Empty one"),
        flagged(["insecureUrl"], "e-u", "Http one"),
      ]),
    );
    expect(within(optionNamed(/GitLab/)).getByText(t("Weak password"))).toBeTruthy();
    expect(within(optionNamed(/Reused one/)).getByText(t("Reused password"))).toBeTruthy();
    expect(within(optionNamed(/Empty one/)).getByText(t("No password"))).toBeTruthy();
    expect(within(optionNamed(/Http one/)).getByText(t("Insecure URL"))).toBeTruthy();
  });

  it("security_chips_follow_the_ui_language", async () => {
    const fake = backend();
    fake.state.settings.language = "ru";
    await open(fake);
    expect(within(optionNamed(/GitLab/)).getByText(ru["Weak password"])).toBeTruthy();
  });

  it("rows_are_draggable_and_carry_only_the_id", async () => {
    await open();
    const dataTransfer = new FakeDataTransfer();
    await fireEvent.dragStart(optionNamed(/GitHub/), { dataTransfer });
    expect(dataTransfer.getData(ENTRY_MIME)).toBe("e-github");
    expect(dataTransfer.types).toEqual([ENTRY_MIME]);
  });
});

describe("header", () => {
  it("list_header_uses_plural_key", async () => {
    // Five entries outside the bin: the four fixtures plus one.
    const fake = backend([{ id: "e-extra", title: "Extra", modified: 1 }]);
    fake.state.settings.language = "ru";
    await open(fake);
    const many = /many \{([^}]*)\}/.exec(ru["%lld items"])?.[1];
    expect(many).toBeTruthy();
    expect(screen.getByTestId("list-count").textContent).toBe((many ?? "").replace("#", "5"));
  });

  it("list_header_singular_in_english", async () => {
    await open();
    navigation.setScope({ kind: "category", category: "deleted" });
    await waitFor(() => expect(screen.getByTestId("list-count").textContent).toBe("1 item"));
  });

  it("search_header_uses_matching_items_key", async () => {
    const { fake } = await open();
    await search(fake, "git");
    await waitFor(() => expect(screen.getByTestId("list-count").textContent).toBe("2 matching items"));
  });
});

describe("search", () => {
  it("search_hit_only_in_notes_or_address_field_still_lists_entry", async () => {
    const fake = backend([
      { id: "e-a", title: "Alpha", notes: "the needle is here", modified: 1 },
      { id: "e-b", title: "Beta", entryType: "address", address: address("Needleville"), modified: 1 },
      // A login that merely carries an address block: the 14 fields count for ADDRESS entries only.
      { id: "e-c", title: "Gamma", address: address("Needleton"), modified: 1 },
    ]);
    await open(fake);
    expect(titles()).toContain("Alpha");
    await search(fake, "needle");
    await waitFor(() => expect(titles()).toEqual(["Alpha", "Beta"]));
  });

  it("search_matches_attachment_names_in_app_list", async () => {
    const fake = backend([{ id: "e-f", title: "Taxes", entryType: "files", attachments: [{ name: "return-2024.pdf", size: 9 }], modified: 1 }]);
    await open(fake);
    await search(fake, "return-2024");
    await waitFor(() => expect(titles()).toEqual(["Taxes"]));
  });

  it("search_matches_username_urls_and_group_name_but_never_the_password", async () => {
    const { fake } = await open();
    await search(fake, "gitlab.com");
    await waitFor(() => expect(titles()).toEqual(["GitLab"]));
    await search(fake, "octo-lab", true);
    await waitFor(() => expect(titles()).toEqual(["GitLab"]));
    // Only the group name ("Personal") matches these: GitLab lives in Work and must drop out.
    await search(fake, "Personal", true);
    await waitFor(() => expect(titles()).toEqual(["GitHub", "Server notes", "Visa"]));
    await search(fake, "Work", true);
    await waitFor(() => expect(titles()).toEqual(["GitLab"]));
    await search(fake, FIXTURE_LOGIN_PASSWORD, true);
    await waitFor(() => expect(options()).toHaveLength(0));
  });

  it("a_search_inside_a_group_covers_the_whole_vault_but_not_the_bin", async () => {
    const { fake } = await open();
    navigation.setScope({ kind: "group", groupId: WORK_GROUP_ID });
    await waitFor(() => expect(titles()).toEqual(["GitLab"]));
    await search(fake, "o");
    await waitFor(() => expect(titles()).toContain("GitHub"));
    expect(titles()).not.toContain("Old forum");
  });

  it("typing_fast_refetches_once_after_the_pause", async () => {
    const { fake } = await open();
    const before = calls(fake, "entryList").length;
    await search(fake, "git");
    await waitFor(() => expect(titles()).toEqual(["GitHub", "GitLab"]));
    const withText = calls(fake, "entryList")
      .slice(before)
      .filter((c) => (c.args.query as { search: string }).search !== "");
    expect(withText).toHaveLength(1);
  });

  it("sort_select_triggers_query_with_modified_sort", async () => {
    const { fake } = await open();
    expect(titles()).toEqual(["GitHub", "GitLab", "Server notes", "Visa"]);
    await userEvent.setup().selectOptions(screen.getByRole("combobox", { name: t("Sort by") }), t("Modified"));
    await waitFor(() => expect((calls(fake, "entryList").at(-1)?.args.query as { sort: string }).sort).toBe("modified"));
    await waitFor(() => expect(titles()).toEqual(["GitHub", "GitLab", "Visa", "Server notes"]));
  });
});

describe("keyboard", () => {
  it("arrow_keys_move_selection_and_load_detail_id", async () => {
    await open();
    const user = userEvent.setup();
    const listbox = screen.getByRole("listbox");
    listbox.focus();
    await user.keyboard("{ArrowDown}");
    expect(navigation.selectedId).toBe("e-github");
    expect(listbox.getAttribute("aria-activedescendant")).toBe(optionNamed(/GitHub/).id);
    await user.keyboard("{ArrowDown}");
    expect(navigation.selectedId).toBe("e-gitlab");
    expect(optionNamed(/GitLab/).getAttribute("aria-selected")).toBe("true");
    expect(optionNamed(/GitHub/).getAttribute("aria-selected")).toBe("false");
    await user.keyboard("{ArrowUp}");
    expect(navigation.selectedId).toBe("e-github");
    await user.keyboard("{End}");
    expect(navigation.selectedId).toBe("e-visa");
    await user.keyboard("{Home}");
    expect(navigation.selectedId).toBe("e-github");
  });

  it("clicking_a_row_selects_it", async () => {
    await open();
    await userEvent.setup().click(optionNamed(/Visa/));
    expect(navigation.selectedId).toBe("e-visa");
  });

  it("enter_moves_focus_to_detail_region", async () => {
    await open();
    const user = userEvent.setup();
    screen.getByRole("listbox").focus();
    await user.keyboard("{ArrowDown}{Enter}");
    expect(document.activeElement).toBe(screen.getByRole("region", { name: t("Details") }));
  });

  it("delete_key_deletes_selected_and_selects_neighbour", async () => {
    const { fake } = await open();
    const user = userEvent.setup();
    await user.click(optionNamed(/GitLab/));
    await user.keyboard("{Delete}");
    await waitFor(() => expect(calls(fake, "entryDelete")).toHaveLength(1));
    expect(calls(fake, "entryDelete")[0]?.args).toEqual({ id: "e-gitlab" });
    // No confirmation for the bin: the neighbour below takes over.
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(navigation.selectedId).toBe("e-notes");
    await waitFor(() => expect(titles()).toEqual(["GitHub", "Server notes", "Visa"]));
  });

  it("delete_of_the_last_row_selects_the_one_above", async () => {
    await open();
    const user = userEvent.setup();
    await user.click(optionNamed(/Visa/));
    await user.keyboard("{Backspace}");
    await waitFor(() => expect(navigation.selectedId).toBe("e-notes"));
  });

  it("delete_key_in_deleted_scope_confirms_permanent_delete", async () => {
    const { fake } = await open();
    navigation.setScope({ kind: "category", category: "deleted" });
    const user = userEvent.setup();
    await user.click(await screen.findByRole("option", { name: /Old forum/ }));
    await user.keyboard("{Delete}");
    const dialog = await screen.findByRole("dialog");
    expect(calls(fake, "entryPurge")).toHaveLength(0);
    await user.click(within(dialog).getByRole("button", { name: t("Cancel") }));
    expect(calls(fake, "entryPurge")).toHaveLength(0);
    await user.keyboard("{Delete}");
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: t("Delete permanently") }));
    await waitFor(() => expect(calls(fake, "entryPurge")).toHaveLength(1));
    expect(calls(fake, "entryPurge")[0]?.args).toEqual({ id: "e-old" });
    expect(calls(fake, "entryDelete")).toHaveLength(0);
  });

  it("mod_c_copies_the_password_of_the_selected_login_only", async () => {
    const { fake } = await open();
    const user = userEvent.setup();
    await user.click(optionNamed(/GitHub/));
    await user.keyboard("{Control>}c{/Control}");
    await waitFor(() => expect(calls(fake, "entryCopy")).toHaveLength(1));
    expect(calls(fake, "entryCopy")[0]?.args).toEqual({ id: "e-github", field: { kind: "password" } });
    await user.click(optionNamed(/Visa/));
    await user.keyboard("{Control>}c{/Control}");
    expect(calls(fake, "entryCopy")).toHaveLength(1);
  });
});

describe("empty states", () => {
  it("empty_state_text_per_scope", async () => {
    const fake = backend();
    fake.state.entries = [];
    await renderMainApp(fake);
    const categories = fixtureSidebar([]).categories.map((c) => c.category);
    const seen = new Map<string, string>();
    const record = async (label: string) => {
      const text = (await screen.findByTestId("empty-state")).textContent ?? "";
      expect(text.trim()).not.toBe("");
      seen.set(label, text);
    };
    for (const category of categories) {
      navigation.setScope({ kind: "category", category: category as Category });
      await waitFor(() => expect(screen.getByTestId("empty-state").getAttribute("data-scope")).toBe(`cat:${category}`));
      await record(`cat:${category}`);
    }
    navigation.setScope({ kind: "group", groupId: WORK_GROUP_ID });
    await waitFor(() => expect(screen.getByTestId("empty-state").getAttribute("data-scope")).toBe("group"));
    await record("group");
    await search(fake, "zzz");
    await waitFor(() => expect(screen.getByTestId("empty-state").getAttribute("data-scope")).toBe("search"));
    await record("search");
    // One text per category, one for a group and one for a search, all different.
    expect(new Set([...seen.values()].map((s) => s.split("\n")[0])).size).toBe(categories.length + 2);
  });

  it("empty_state_headings_are_catalog_keys", async () => {
    const fake = backend();
    fake.state.entries = [];
    await renderMainApp(fake);
    const heading = (await screen.findByTestId("empty-state")).querySelector("[data-part='heading']")?.textContent ?? "";
    expect(Object.keys(en)).toContain(heading);
  });

  it("empty_state_for_search_mentions_query_free_text", async () => {
    const { fake } = await open();
    await search(fake, "zzz-nothing");
    const state = await screen.findByTestId("empty-state");
    expect(state.textContent).toContain(t("No results"));
    expect(state.textContent).toContain("zzz-nothing");
  });
});

describe("context menu", () => {
  const menuItem = (name: string) => screen.getByRole("menuitem", { name });

  it("context_menu_copy_password_calls_entry_copy_with_password_field_ref", async () => {
    const { fake } = await open();
    await fireEvent.contextMenu(optionNamed(/GitHub/));
    await userEvent.setup().click(menuItem(t("Copy password")));
    await waitFor(() => expect(calls(fake, "entryCopy")).toHaveLength(1));
    expect(calls(fake, "entryCopy")[0]?.args).toEqual({ id: "e-github", field: { kind: "password" } });
  });

  it("shows_toast_without_value", async () => {
    await open();
    await fireEvent.contextMenu(optionNamed(/GitHub/));
    await userEvent.setup().click(menuItem(t("Copy password")));
    await waitFor(() => expect(toasts.items.map((x) => x.text)).toEqual([t("Copied")]));
    expect(document.body.textContent).not.toContain(FIXTURE_LOGIN_PASSWORD);
  });

  it("copy_username_asks_for_the_username_field", async () => {
    const { fake } = await open();
    await fireEvent.contextMenu(optionNamed(/GitHub/));
    await userEvent.setup().click(menuItem(t("Copy username")));
    await waitFor(() => expect(calls(fake, "entryCopy")[0]?.args.field).toEqual({ kind: "username" }));
  });

  it("shift_f10_opens_the_menu_for_the_selected_row_and_escape_returns_focus", async () => {
    await open();
    const user = userEvent.setup();
    const listbox = screen.getByRole("listbox");
    await user.click(optionNamed(/GitHub/));
    listbox.focus();
    await user.keyboard("{Shift>}{F10}{/Shift}");
    expect(await screen.findByRole("menu")).toBeTruthy();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(listbox);
  });

  it("open_website_only_for_http_https", async () => {
    const opened: unknown[] = [];
    const plugins: Plugins = { "plugin:opener|open_url": (p) => (opened.push(p), null) };
    const fake = backend(
      [
        { id: "e-js", title: "Script", urls: ["javascript:alert(1)"], modified: 1 },
        { id: "e-ftp", title: "Ftp", urls: ["ftp://example.com"], modified: 1 },
        { id: "e-http", title: "Plain", urls: ["http://example.com/x"], modified: 1 },
      ],
      plugins,
    );
    await open(fake);
    const user = userEvent.setup();
    const choose = async (name: RegExp) => {
      await fireEvent.contextMenu(optionNamed(name));
      await user.click(menuItem(t("Open website")));
    };
    await choose(/Script/);
    await choose(/Ftp/);
    await new Promise((r) => setTimeout(r, 20));
    expect(opened).toEqual([]);
    expect(toasts.items.length).toBeGreaterThan(0);
    await choose(/GitHub/);
    await choose(/Plain/);
    await waitFor(() => expect(opened).toHaveLength(2));
    expect((opened[0] as { url: string }).url).toBe("https://github.com/");
    expect((opened[1] as { url: string }).url).toBe("http://example.com/x");
  });

  it("website_and_copy_items_are_disabled_for_non_logins", async () => {
    await open();
    await fireEvent.contextMenu(optionNamed(/Visa/));
    for (const name of [t("Copy username"), t("Copy password"), t("Open website")]) {
      expect(menuItem(name).hasAttribute("disabled") || menuItem(name).getAttribute("aria-disabled") === "true").toBe(true);
    }
  });

  it("favorite_toggle_calls_entry_set_favorite", async () => {
    const { fake } = await open();
    const user = userEvent.setup();
    await fireEvent.contextMenu(optionNamed(/GitHub/));
    await user.click(menuItem(t("Remove from Favorites")));
    await waitFor(() => expect(calls(fake, "entrySetFavorite")).toHaveLength(1));
    expect(calls(fake, "entrySetFavorite")[0]?.args).toEqual({ id: "e-github", favorite: false });
    await fireEvent.contextMenu(optionNamed(/GitLab/));
    await user.click(menuItem(t("Add to Favorites")));
    await waitFor(() => expect(calls(fake, "entrySetFavorite")).toHaveLength(2));
    expect(calls(fake, "entrySetFavorite")[1]?.args).toEqual({ id: "e-gitlab", favorite: true });
  });

  it("move_to_opens_the_group_picker_and_moves", async () => {
    const { fake } = await open();
    const user = userEvent.setup();
    await fireEvent.contextMenu(optionNamed(/GitHub/));
    await user.click(menuItem(t("Move to…")));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("treeitem", { name: /Work/ }));
    await waitFor(() => expect(calls(fake, "entryMove")).toHaveLength(1));
    expect(calls(fake, "entryMove")[0]?.args).toEqual({ id: "e-github", groupId: WORK_GROUP_ID });
  });

  it("delete_item_moves_to_the_bin_and_deleted_rows_offer_permanent_delete", async () => {
    const { fake } = await open();
    const user = userEvent.setup();
    await fireEvent.contextMenu(optionNamed(/GitHub/));
    await user.click(menuItem(t("Delete")));
    await waitFor(() => expect(calls(fake, "entryDelete")[0]?.args).toEqual({ id: "e-github" }));
    navigation.setScope({ kind: "category", category: "deleted" });
    await fireEvent.contextMenu(await screen.findByRole("option", { name: /Old forum/ }));
    expect(screen.queryByRole("menuitem", { name: t("Delete") })).toBeNull();
    expect(menuItem(t("Delete permanently"))).toBeTruthy();
  });
});

describe("empty the bin", () => {
  it("empty_bin_confirms_then_reports_count_toast", async () => {
    const fake = backend([
      { id: "b1", title: "Bin one", groupId: BIN_GROUP_ID, inRecycleBin: true, modified: 1 },
      { id: "b2", title: "Bin two", groupId: BIN_GROUP_ID, inRecycleBin: true, modified: 1 },
    ]);
    await open(fake);
    expect(screen.queryByRole("button", { name: t("Empty Deleted") })).toBeNull();
    navigation.setScope({ kind: "category", category: "deleted" });
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: t("Empty Deleted") }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(t("Empty Deleted? This cannot be undone."))).toBeTruthy();
    expect(calls(fake, "entryEmptyBin")).toHaveLength(0);
    await user.click(within(dialog).getByRole("button", { name: t("Empty Deleted") }));
    await waitFor(() => expect(calls(fake, "entryEmptyBin")).toHaveLength(1));
    // Three entries were in the bin: the fixture's own plus the two added here.
    await waitFor(() => expect(toasts.items.map((x) => x.text)).toEqual([t("%lld items deleted", 3)]));
    expect(toasts.items[0]?.text).toBe("3 items deleted");
    await waitFor(() => expect(screen.getByTestId("empty-state")).toBeTruthy());
  });
});

describe("size and accessibility", () => {
  it("renders_2000_rows_with_one_fetch_and_one_selected_row", async () => {
    // R26: no wall-clock budget; the DOM size and the number of fetches are what is pinned.
    const many: EntrySpec[] = Array.from({ length: 2000 }, (_, i) => ({
      id: `bulk-${i}`,
      title: `Bulk ${String(i).padStart(4, "0")}`,
      modified: i,
    }));
    const fake = backend(many);
    const expected = fake.state.entries.filter((e) => !e.summary.inRecycleBin).length;
    await renderMainApp(fake);
    await waitFor(() => expect(options()).toHaveLength(expected));
    expect(calls(fake, "entryList")).toHaveLength(1);
    const user = userEvent.setup();
    screen.getByRole("listbox").focus();
    await user.keyboard("{ArrowDown}{ArrowDown}");
    expect(options().filter((o) => o.getAttribute("aria-selected") === "true")).toHaveLength(1);
    expect(calls(fake, "entryList")).toHaveLength(1);
  });

  it("list_passes_axe", async () => {
    const { container } = await open();
    await expectNoAxeViolations(container);
  });

  it("empty_list_passes_axe", async () => {
    const fake = backend();
    fake.state.entries = [];
    const { container } = await renderMainApp(fake);
    await screen.findByTestId("empty-state");
    await expectNoAxeViolations(container);
  });
});

describe("edge cases", () => {
  it("no_empty_state_before_the_first_list_answer", async () => {
    const fake = backend();
    const real = fake.handlers.entryList;
    let release: () => void = () => undefined;
    const gate = new Promise<void>((r) => (release = r));
    fake.handlers.entryList = async (args) => {
      await gate;
      return real(args);
    };
    await renderMainApp(fake);
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByTestId("empty-state")).toBeNull();
    release();
    await screen.findByRole("listbox");
  });

  it("clicking_a_row_puts_focus_on_the_listbox", async () => {
    await open();
    await userEvent.setup().click(optionNamed(/Visa/));
    const listbox = screen.getByRole("listbox");
    expect(document.activeElement).toBe(listbox);
    expect(listbox.getAttribute("aria-activedescendant")).toBe(optionNamed(/Visa/).id);
  });

  it("keys_with_a_stale_selected_id_do_nothing", async () => {
    const { fake } = await open();
    const user = userEvent.setup();
    await user.click(optionNamed(/Visa/));
    await search(fake, "git");
    await waitFor(() => expect(titles()).toEqual(["GitHub", "GitLab"]));
    expect(navigation.selectedId).toBe("e-visa");
    const listbox = screen.getByRole("listbox");
    expect(listbox.hasAttribute("aria-activedescendant")).toBe(false);
    listbox.focus();
    await user.keyboard("{Enter}{Delete}{Control>}c{/Control}");
    expect(calls(fake, "entryDelete")).toHaveLength(0);
    expect(calls(fake, "entryCopy")).toHaveLength(0);
    expect(screen.queryByRole("dialog")).toBeNull();
    // Down starts from the top instead of throwing.
    await user.keyboard("{ArrowDown}");
    expect(navigation.selectedId).toBe("e-github");
  });

  it("move_to_is_disabled_for_rows_in_the_bin", async () => {
    await open();
    navigation.setScope({ kind: "category", category: "deleted" });
    await fireEvent.contextMenu(await screen.findByRole("option", { name: /Old forum/ }));
    expect(screen.getByRole("menuitem", { name: t("Move to…") }).getAttribute("aria-disabled")).toBe("true");
  });

  it("escape_on_the_purge_dialog_purges_nothing", async () => {
    const { fake } = await open();
    navigation.setScope({ kind: "category", category: "deleted" });
    const user = userEvent.setup();
    await user.click(await screen.findByRole("option", { name: /Old forum/ }));
    await user.keyboard("{Delete}");
    await screen.findByRole("dialog");
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(calls(fake, "entryPurge")).toHaveLength(0);
    expect(optionNamed(/Old forum/)).toBeTruthy();
  });
});
