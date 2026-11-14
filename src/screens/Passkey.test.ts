// The passkey surfaces: the detail row and popover, the editor's deferred removal, the category.
import { screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";
import { formatDate } from "../lib/i18n/format";
import { setLanguage, t } from "../lib/i18n/index.svelte";
import { navigation } from "../lib/stores/navigation.svelte";
import { expectNoAxeViolations } from "../test/axe";
import { backendWith, openEdit, user } from "../test/editorHelpers";
import { FIXTURE_PASSKEY_CREDENTIAL_ID, sessionStatus, type EntrySpec } from "../test/fixtures";
import { calls, renderMainApp } from "../test/mainApp";
import type { FakeBackend } from "../test/fakeBackend";

afterEach(() => setLanguage("en"));

const CREATED = 1_700_000_000_000;
const PK: EntrySpec = {
  id: "e-pk",
  title: "Passkeyed",
  modified: 5,
  passkey: { relyingParty: "example.org", username: "ann@example.org", created: CREATED, usable: true },
};
const pane = () => screen.getByRole("region", { name: t("Details") });
const text = () => document.body.textContent ?? "";

async function openDetail(fake: FakeBackend, id: string, title: string) {
  const app = await renderMainApp(fake);
  await screen.findByRole("listbox");
  navigation.select(id);
  await screen.findByRole("heading", { name: title });
  return app;
}
const showButton = (scope: HTMLElement) => within(scope).getByRole("button", { name: t("Show passkey details") });
const detailCalls = (fake: FakeBackend) => calls(fake, "entryPasskeyDetails");

describe("detail row", () => {
  it("detail_passkey_row_shows_added_date_and_show_button", async () => {
    await openDetail(backendWith([PK]), "e-pk", "Passkeyed");
    const row = within(pane()).getByRole("group", { name: "Passkey" });
    expect(row.textContent).toContain(`Added ${formatDate(CREATED)}`);
    expect(showButton(row).textContent?.trim()).toBe("Show");
  });

  it("detail_passkey_row_without_created_says_added_only", async () => {
    const fake = backendWith([{ ...PK, passkey: { ...PK.passkey!, created: null } }]);
    await openDetail(fake, "e-pk", "Passkeyed");
    const row = within(pane()).getByRole("group", { name: "Passkey" });
    expect(row.textContent).toContain("Added");
    expect(row.textContent).not.toMatch(/\d/);
  });

  it("show_opens_popover_with_rp_username_created_and_credential_id", async () => {
    const fake = backendWith([PK]);
    await openDetail(fake, "e-pk", "Passkeyed");
    await user().click(showButton(pane()));
    const popover = await screen.findByRole("region", { name: "Passkey" });
    expect(popover.textContent).toContain("Site");
    expect(popover.textContent).toContain("example.org");
    expect(popover.textContent).toContain("ann@example.org");
    expect(popover.textContent).toContain(formatDate(CREATED));
    expect(popover.textContent).toContain(FIXTURE_PASSKEY_CREDENTIAL_ID);
    expect(detailCalls(fake)).toHaveLength(1);
  });

  it("popover_footnote_only_in_detail_context", async () => {
    const fake = backendWith([PK]);
    await openEdit(fake, "e-pk");
    await user().click(showButton(document.body));
    const popover = await screen.findByRole("region", { name: "Passkey" });
    expect(popover.textContent).toContain(FIXTURE_PASSKEY_CREDENTIAL_ID);
    expect(popover.textContent).not.toContain(t("Remove this passkey by editing the entry."));
    navigation.closeEditor();
    await screen.findByRole("heading", { name: "Passkeyed" });
    await user().click(showButton(pane()));
    expect((await screen.findByRole("region", { name: "Passkey" })).textContent).toContain(
      t("Remove this passkey by editing the entry."),
    );
  });

  it("popover_for_keepassxc_passkey_says_unknown_created_outside_mypass", async () => {
    const fake = backendWith([{ ...PK, passkey: { ...PK.passkey!, created: null, username: "" } }]);
    await openDetail(fake, "e-pk", "Passkeyed");
    await user().click(showButton(pane()));
    const popover = await screen.findByRole("region", { name: "Passkey" });
    expect(popover.textContent).toContain(t("Unknown — created outside MyPass"));
    expect(popover.textContent).not.toContain(t("Username"));
  });

  it("webview_dom_never_contains_credential_secret", async () => {
    const fake = backendWith([PK]);
    await openDetail(fake, "e-pk", "Passkeyed");
    expect(detailCalls(fake)).toHaveLength(0);
    expect(text()).not.toContain(FIXTURE_PASSKEY_CREDENTIAL_ID);
    expect(text()).not.toContain("PRIVATE KEY");
    expect(JSON.stringify(navigation.editModel)).not.toContain(FIXTURE_PASSKEY_CREDENTIAL_ID);
    await user().click(showButton(pane()));
    await waitFor(() => expect(text()).toContain(FIXTURE_PASSKEY_CREDENTIAL_ID));
    expect(text()).not.toContain("PRIVATE KEY");
  });

  it("lock_event_closes_popover_and_clears_details", async () => {
    const fake = backendWith([PK]);
    await openDetail(fake, "e-pk", "Passkeyed");
    await user().click(showButton(pane()));
    await screen.findByRole("region", { name: "Passkey" });
    await fake.emit("sessionChanged", sessionStatus("locked"));
    await waitFor(() => expect(screen.queryByRole("region", { name: "Passkey" })).toBeNull());
    expect(text()).not.toContain(FIXTURE_PASSKEY_CREDENTIAL_ID);
    expect(text()).not.toContain("example.org");
  });

  it("damaged_passkey_detail_shows_label_and_no_show_button", async () => {
    const fake = backendWith([{ ...PK, passkey: { ...PK.passkey!, usable: false } }]);
    await openDetail(fake, "e-pk", "Passkeyed");
    const row = within(pane()).getByRole("group", { name: "Passkey" });
    expect(row.textContent).toContain(t("Damaged passkey"));
    expect(within(row).queryByRole("button")).toBeNull();
  });

  it("passkey_fields_not_rendered_as_custom_fields", async () => {
    const fake = backendWith([PK]);
    await openDetail(fake, "e-pk", "Passkeyed");
    expect(text()).not.toContain("KPEX_PASSKEY");
    await user().click(within(pane()).getByRole("button", { name: "Edit" }));
    await screen.findByRole("button", { name: "Save" });
    expect(text()).not.toContain("KPEX_PASSKEY");
  });

  it("passkey_surfaces_pass_axe", async () => {
    const fake = backendWith([PK]);
    const app = await openDetail(fake, "e-pk", "Passkeyed");
    await user().click(showButton(pane()));
    await screen.findByRole("region", { name: "Passkey" });
    await expectNoAxeViolations(app.container);
  });
});

describe("editor", () => {
  const savedModel = (fake: FakeBackend) => calls(fake, "entrySave")[0]?.args.model;
  const editorRow = () => screen.getByRole("group", { name: "Passkey" });
  const confirmRemoval = async () => {
    await user().click(within(editorRow()).getByRole("button", { name: "Remove" }));
    const dialog = await screen.findByRole("dialog", { name: t("Remove passkey?") });
    expect(dialog.textContent).toContain("history");
    await user().click(within(dialog).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  };

  it("editor_remove_marks_deferred_removal_with_undo", async () => {
    const fake = backendWith([PK]);
    await openEdit(fake, "e-pk");
    await confirmRemoval();
    expect(editorRow().textContent).toContain(t("Passkey will be removed when you save"));
    expect(within(editorRow()).queryByRole("button", { name: "Remove" })).toBeNull();
    // Show still works in the removal state, then Undo restores the row.
    await user().click(showButton(editorRow()));
    expect((await screen.findByRole("region", { name: "Passkey" })).textContent).toContain(FIXTURE_PASSKEY_CREDENTIAL_ID);
    await user().click(within(editorRow()).getByRole("button", { name: "Undo" }));
    expect(editorRow().textContent).not.toContain(t("Passkey will be removed when you save"));
    expect(within(editorRow()).getByRole("button", { name: "Remove" })).toBeTruthy();
    expect(navigation.editModel?.removePasskey).toBe(false);
  });

  it("editor_save_sends_remove_passkey_true_only_after_remove", async () => {
    const fake = backendWith([PK]);
    await openEdit(fake, "e-pk");
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "entrySave")).toHaveLength(1));
    expect(savedModel(fake).removePasskey).toBe(false);

  });

  it("editor_save_after_remove_sends_the_flag_and_the_row_is_gone_after_the_reread", async () => {
    const fake2 = backendWith([PK]);
    await openEdit(fake2, "e-pk");
    await confirmRemoval();
    const ref = navigation.editModel?.passkeyRef;
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake2, "entrySave")).toHaveLength(1));
    expect(savedModel(fake2)).toMatchObject({ removePasskey: true, passkeyRef: ref });
    // The detail is re-read after the save: the row is gone.
    await screen.findByRole("heading", { name: "Passkeyed" });
    await waitFor(() => expect(within(pane()).queryByRole("group", { name: "Passkey" })).toBeNull());
  });

  it("editor_cancel_leaves_passkey_untouched", async () => {
    const fake = backendWith([PK]);
    await openEdit(fake, "e-pk");
    await confirmRemoval();
    await user().click(screen.getByRole("button", { name: "Cancel" }));
    await user().click(await screen.findByRole("button", { name: "Discard" }));
    await screen.findByRole("heading", { name: "Passkeyed" });
    expect(calls(fake, "entrySave")).toHaveLength(0);
    expect(within(pane()).getByRole("group", { name: "Passkey" })).toBeTruthy();
  });

  it("editor_without_passkey_has_no_passkey_row", async () => {
    await openEdit(backendWith([{ id: "e-plain", title: "Plain", modified: 1 }]), "e-plain");
    expect(screen.queryByRole("group", { name: "Passkey" })).toBeNull();
  });

  it("editor_damaged_passkey_offers_only_remove", async () => {
    const fake = backendWith([{ ...PK, passkey: { ...PK.passkey!, usable: false } }]);
    await openEdit(fake, "e-pk");
    expect(editorRow().textContent).toContain(t("Damaged passkey"));
    expect(within(editorRow()).queryByRole("button", { name: t("Show passkey details") })).toBeNull();
    await confirmRemoval();
    expect(editorRow().textContent).toContain(t("Passkey will be removed when you save"));
  });

  it("editor_dirty_after_remove_marks_changes", async () => {
    await openEdit(backendWith([PK]), "e-pk");
    await confirmRemoval();
    expect(navigation.editorDirty?.()).toBe(true);
  });

  it("editor_passkey_row_passes_axe", async () => {
    const app = await openEdit(backendWith([PK]), "e-pk");
    await expectNoAxeViolations(app.container);
    await confirmRemoval();
    await expectNoAxeViolations(app.container);
  });
});

describe("passkeys category", () => {
  it("passkeys_sidebar_category_lists_only_usable_passkeys", async () => {
    const fake = backendWith([
      PK,
      { id: "e-bad", title: "Broken", modified: 4, passkey: { ...PK.passkey!, usable: false } },
      { id: "e-plain", title: "Plain", modified: 3 },
    ]);
    await renderMainApp(fake);
    await screen.findByRole("listbox");
    await user().click(screen.getByRole("treeitem", { name: /^Passkeys/ }));
    await waitFor(() => expect(screen.queryByRole("option", { name: /Plain/ })).toBeNull());
    expect(screen.getByRole("option", { name: /Passkeyed/ })).toBeTruthy();
    expect(screen.queryByRole("option", { name: /Broken/ })).toBeNull();
    expect(screen.queryByRole("option", { name: /Plain/ })).toBeNull();
    // Ordering and count agree: the badge counts what the list shows.
    const shown = within(screen.getByRole("listbox")).getAllByRole("option").length;
    expect(shown).toBe(fake.state.entries.filter((e) => e.summary.hasPasskey && !e.summary.inRecycleBin).length);
    expect(screen.getByRole("treeitem", { name: /^Passkeys/ }).textContent).toContain(String(shown));
  });
});
