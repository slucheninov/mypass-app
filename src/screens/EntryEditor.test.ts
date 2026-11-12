import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../lib/api";
import { setLanguage, t } from "../lib/i18n/index.svelte";
import { navigation } from "../lib/stores/navigation.svelte";
import { toasts } from "../lib/stores/toasts.svelte";
import { expectNoAxeViolations } from "../test/axe";
import { backendWith, openEdit, openNew, user } from "../test/editorHelpers";
import { WORK_GROUP_ID, sessionStatus } from "../test/fixtures";
import { calls, renderMainApp } from "../test/mainApp";
import LoginFormHarness from "../test/LoginFormHarness.svelte";
import { renderWithBackend } from "../test/render";
import { createFakeBackend } from "../test/fakeBackend";

afterEach(() => {
  vi.useRealTimers();
  setLanguage("en");
});

const field = (label: string) => screen.getByRole("textbox", { name: label }) as HTMLInputElement;
const savedModel = (fake: { calls: { command: string; args: Record<string, any> }[] }) =>
  fake.calls.filter((c) => c.command === "entrySave")[0]?.args.model;

describe("login form", () => {
  it("new_login_form_defaults_group_from_scope", async () => {
    const fake = backendWith();
    await renderMainApp(fake);
    await screen.findByRole("listbox");
    navigation.setScope({ kind: "group", groupId: WORK_GROUP_ID });
    await user().click(await screen.findByRole("button", { name: "New Entry" }));
    await user().click(await screen.findByRole("menuitem", { name: "New Login" }));
    expect(await screen.findByRole("heading", { name: "New Login" })).toBeTruthy();
    expect(calls(fake, "entryNewModel")[0]?.args.groupId).toBe(WORK_GROUP_ID);
    expect(screen.getByRole("button", { name: "Work" })).toBeTruthy();
  });

  it("save_calls_entry_save_with_trimmed_title_and_nonempty_urls", async () => {
    const fake = backendWith([{ id: "e-bank", title: "Old", urls: ["https://a.test", ""], username: "me", modified: 1 }]);
    await openEdit(fake, "e-bank");
    const title = field("Title");
    await user().clear(title);
    await user().type(title, "  Bank ");
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "entrySave")).toHaveLength(1));
    const model = savedModel(fake);
    expect(model.title).toBe("Bank");
    expect(model.urls).toEqual(["https://a.test"]);
  });

  it("login_without_title_blocks_save", async () => {
    const fake = backendWith();
    await openNew(fake, "login");
    await user().click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(calls(fake, "entrySave")).toHaveLength(0);
    expect(navigation.editModel).not.toBeNull();
  });

  it("address_without_title_saves_and_the_backend_applies_the_label_fallback", async () => {
    const fake = backendWith();
    await openNew(fake, "address");
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(navigation.editModel).toBeNull());
    expect(calls(fake, "entrySave")).toHaveLength(1);
    expect(fake.state.entries.at(-1)?.summary.title).toBe("Home");
  });

  it("password_field_accepts_value_from_generator_callback", async () => {
    const fake = createFakeBackend({ session: "unlocked" });
    const model = await (async () => {
      fake.install();
      return api.entryNewModel("login", null);
    })();
    renderWithBackend(LoginFormHarness, { model }, fake);
    await user().click(screen.getByRole("button", { name: "Generate" }));
    await user().click(screen.getByRole("button", { name: "Use abc" }));
    expect((screen.getByLabelText("Password") as HTMLInputElement).value).toBe("abc");
  });

  it("totp_and_other_invalid_input_show_one_inline_form_error_without_a_toast", async () => {
    const fake = backendWith();
    await openNew(fake, "login");
    await user().type(field("Title"), "Bank");
    await user().type(screen.getByLabelText("One-time password"), "not-a-secret");
    fake.state.saveFailure = "invalidInput";
    await user().click(screen.getByRole("button", { name: "Save" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain(t("Invalid input"));
    expect(toasts.items).toHaveLength(0);
    expect(navigation.editModel).not.toBeNull();
  });

  it("custom_fields_add_remove_and_protected_toggle", async () => {
    const fake = backendWith();
    await openNew(fake, "login");
    await user().type(field("Title"), "Bank");
    await user().click(screen.getByRole("button", { name: "Add field" }));
    await user().type(screen.getByRole("textbox", { name: "Field name" }), "PIN");
    await user().type(screen.getByLabelText("Field value"), "1234");
    expect(screen.getByLabelText("Field value").getAttribute("type")).toBe("text");
    await user().click(screen.getByRole("switch", { name: "Protected" }));
    expect(screen.getByLabelText("Field value").getAttribute("type")).toBe("password");
    await user().click(screen.getByRole("button", { name: "Add field" }));
    expect(screen.getAllByRole("textbox", { name: "Field name" })).toHaveLength(2);
    await user().click(screen.getAllByRole("button", { name: "Remove" })[1]!);
    expect(screen.getAllByRole("textbox", { name: "Field name" })).toHaveLength(1);
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "entrySave")).toHaveLength(1));
    expect(savedModel(fake).customFields).toEqual([{ key: "PIN", value: "1234", protected: true }]);
  });

  it("favorite_toggle_and_group_picker_change_the_saved_model", async () => {
    const fake = backendWith();
    await openNew(fake, "login");
    await user().type(field("Title"), "Bank");
    await user().click(screen.getByRole("switch", { name: "Favorite" }));
    await user().click(screen.getByRole("button", { name: "Personal" }));
    const picker = await screen.findByRole("dialog", { name: "Group" });
    await user().click(picker.querySelector(`[role="treeitem"][data-id="${WORK_GROUP_ID}"]`)!);
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "entrySave")).toHaveLength(1));
    expect(savedModel(fake)).toMatchObject({ favorite: true, groupId: WORK_GROUP_ID });
  });
});

describe("other entry types", () => {
  it("address_label_options_home_work_other", async () => {
    const fake = backendWith();
    await openNew(fake, "address");
    const select = screen.getByRole("combobox", { name: "Label" });
    expect(within(select).getAllByRole("option").map((o) => o.textContent)).toEqual(["Home", "Work", "Other"]);
    expect(screen.getAllByRole("textbox").length).toBeGreaterThanOrEqual(13);
  });

  it("address_form_has_fourteen_fields_that_map_to_the_model", async () => {
    const fake = backendWith();
    await openNew(fake, "address");
    const labels = ["Full Name", "Given Name", "Family Name", "Organization", "Address Line 1", "Address Line 2", "City", "Region", "Postal Code", "Country Code", "Country", "Phone", "Email"];
    for (const label of labels) await user().type(field(label), `v-${label}`);
    await user().selectOptions(screen.getByRole("combobox", { name: "Label" }), "Work");
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "entrySave")).toHaveLength(1));
    const address = savedModel(fake).address;
    expect(Object.keys(address)).toHaveLength(14);
    expect(address).toMatchObject({ label: "Work", fullName: "v-Full Name", countryCode: "v-Country Code", countryName: "v-Country", email: "v-Email" });
  });

  it("card_form_fields_map_to_card_edit", async () => {
    const fake = backendWith();
    await openNew(fake, "card");
    await user().type(field("Title"), "Visa");
    await user().type(field("Cardholder name"), "A B");
    await user().type(screen.getByLabelText("Number"), "4111");
    await user().type(field("Expiry"), "12/30");
    await user().type(screen.getByLabelText("Security code"), "123");
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "entrySave")).toHaveLength(1));
    expect(savedModel(fake).card).toEqual({ holder: "A B", number: "4111", expiry: "12/30", cvv: "123" });
  });
});

describe("replacing an open editor", () => {
  async function chooseNew(item: string) {
    await user().click(screen.getByRole("button", { name: "New Entry" }));
    await user().click(await screen.findByRole("menuitem", { name: item }));
  }

  it("new_editor_over_dirty_editor_asks_discard_confirmation", async () => {
    await openNew(backendWith(), "login");
    await user().type(field("Title"), "unsaved");
    await chooseNew("New Address");
    const dialog = await screen.findByRole("dialog", { name: "Discard changes?" });
    expect(navigation.editModel?.title).toBe("unsaved");
    expect(navigation.editModel?.entryType).toBe("login");
    await user().click(within(dialog).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(navigation.editModel?.entryType).toBe("address"));
    expect(screen.getByRole("heading", { name: "New Address" })).toBeTruthy();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("keep_editing_leaves_the_dirty_editor_and_drops_the_request", async () => {
    await openNew(backendWith(), "login");
    await user().type(field("Title"), "unsaved");
    await chooseNew("New Address");
    const dialog = await screen.findByRole("dialog", { name: "Discard changes?" });
    await user().click(within(dialog).getByRole("button", { name: "Keep editing" }));
    expect(navigation.editModel?.entryType).toBe("login");
    expect(navigation.pendingModel).toBeNull();
    expect(field("Title").value).toBe("unsaved");
    // A later plain cancel must not open the dropped request.
    await user().click(screen.getByRole("button", { name: "Cancel" }));
    await user().click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Discard" }));
    expect(navigation.editModel).toBeNull();
  });

  it("a_clean_editor_is_replaced_at_once", async () => {
    await openNew(backendWith(), "login");
    await chooseNew("New Address");
    await waitFor(() => expect(navigation.editModel?.entryType).toBe("address"));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("edit_over_a_dirty_editor_goes_through_the_same_request", async () => {
    await openNew(backendWith(), "login");
    await user().type(field("Title"), "unsaved");
    const other = await api.entryNewModel("card", null);
    navigation.requestOpenEditor(other);
    expect(await screen.findByRole("dialog", { name: "Discard changes?" })).toBeTruthy();
    expect(navigation.editModel?.entryType).toBe("login");
  });
});

describe("editor lifecycle", () => {
  it("edit_model_cleared_on_cancel_save_and_lock", async () => {
    // Cancel (clean)
    let fake = backendWith();
    await openNew(fake, "login");
    await user().click(screen.getByRole("button", { name: "Cancel" }));
    expect(navigation.editModel).toBeNull();
    expect(navigation.presentation).toBe("browse");
    // Save
    navigation.openEditor(await api.entryNewModel("login", null));
    await user().type(await screen.findByRole("textbox", { name: "Title" }), "Bank");
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(navigation.editModel).toBeNull());
    // Lock
    navigation.openEditor(await api.entryNewModel("login", null));
    await screen.findByRole("textbox", { name: "Title" });
    await fake.emit("sessionChanged", sessionStatus("locked"));
    await waitFor(() => expect(navigation.editModel).toBeNull());
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
    fake = backendWith();
  });

  it("the_password_is_gone_from_the_store_and_dom_after_close", async () => {
    const fake = backendWith([{ id: "e-bank", title: "Bank", password: "hunter2-secret", modified: 1 }]);
    await openEdit(fake, "e-bank");
    expect((screen.getByLabelText("Password") as HTMLInputElement).value).toBe("hunter2-secret");
    await user().click(screen.getByRole("button", { name: "Cancel" }));
    expect(navigation.editModel).toBeNull();
    expect(JSON.stringify(navigation)).not.toContain("hunter2-secret");
    expect(document.body.innerHTML).not.toContain("hunter2-secret");
  });

  it("dirty_cancel_asks_confirmation_and_clean_cancel_does_not", async () => {
    const fake = backendWith();
    await openNew(fake, "login");
    await user().click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(navigation.editModel).toBeNull();

    navigation.openEditor(await api.entryNewModel("login", null));
    await user().type(await screen.findByRole("textbox", { name: "Title" }), "x");
    await user().click(screen.getByRole("button", { name: "Cancel" }));
    const dialog = await screen.findByRole("dialog", { name: "Discard changes?" });
    await user().click(within(dialog).getByRole("button", { name: "Keep editing" }));
    expect(navigation.editModel).not.toBeNull();
    await user().click(screen.getByRole("button", { name: "Cancel" }));
    await user().click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Discard" }));
    expect(navigation.editModel).toBeNull();
    expect(calls(fake, "entrySave")).toHaveLength(0);
  });

  it("typing_and_deleting_back_to_the_original_is_not_dirty", async () => {
    const fake = backendWith();
    await openNew(fake, "login");
    await user().type(field("Title"), "x");
    await user().clear(field("Title"));
    await user().click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(navigation.editModel).toBeNull();
  });

  it("mod_enter_saves_and_escape_cancels", async () => {
    const fake = backendWith();
    await openNew(fake, "login");
    await fireEvent.keyDown(field("Title"), { key: "Escape" });
    expect(navigation.editModel).toBeNull();

    navigation.openEditor(await api.entryNewModel("login", null));
    const title = await screen.findByRole("textbox", { name: "Title" });
    await user().type(title, "Bank");
    await fireEvent.keyDown(title, { key: "Enter", metaKey: true });
    await waitFor(() => expect(calls(fake, "entrySave")).toHaveLength(1));
    await waitFor(() => expect(navigation.editModel).toBeNull());
  });

  it("saving_disables_buttons_and_prevents_double_submit", async () => {
    const fake = backendWith();
    let release: () => void = () => undefined;
    const gate = new Promise<void>((r) => (release = r));
    const original = fake.handlers.entrySave;
    fake.handlers.entrySave = async (args) => {
      await gate;
      return original(args);
    };
    await openNew(fake, "login");
    await user().type(field("Title"), "Bank");
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true));
    expect((screen.getByRole("button", { name: "Cancel" }) as HTMLButtonElement).disabled).toBe(true);
    await fireEvent.keyDown(field("Title"), { key: "Enter", ctrlKey: true });
    release();
    await waitFor(() => expect(navigation.editModel).toBeNull());
    expect(calls(fake, "entrySave")).toHaveLength(1);
  });

  it("successful_save_selects_saved_id_and_returns_to_browse", async () => {
    const fake = backendWith();
    await openNew(fake, "login");
    await user().type(field("Title"), "Bank");
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(navigation.presentation).toBe("browse"));
    const created = fake.state.entries.at(-1)!;
    expect(navigation.selectedId).toBe(created.summary.id);
    expect(await screen.findByRole("heading", { name: "Bank" })).toBeTruthy();
  });

  it("an_inline_save_error_receives_focus", async () => {
    const fake = backendWith();
    await openNew(fake, "login");
    await user().type(field("Title"), "Bank");
    fake.state.saveFailure = "invalidInput";
    await user().click(screen.getByRole("button", { name: "Save" }));
    const alert = await screen.findByRole("alert");
    await waitFor(() => expect(document.activeElement).toBe(alert));
  });

  it("other_save_failures_go_to_a_toast_and_keep_the_editor", async () => {
    const fake = backendWith();
    await openNew(fake, "login");
    await user().type(field("Title"), "Bank");
    fake.state.saveFailure = "io";
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(toasts.items.length).toBe(1));
    expect(navigation.editModel).not.toBeNull();
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("selecting_another_row_keeps_the_editor_open", async () => {
    const fake = backendWith();
    await openNew(fake, "login");
    await user().type(field("Title"), "Bank");
    navigation.select("e-github");
    expect(navigation.editModel).not.toBeNull();
    expect((field("Title")).value).toBe("Bank");
  });

  it("editor_passes_axe", async () => {
    const fake = backendWith();
    fake.state.drafts["e-github"] = [{ id: "d1", savedAt: 1_700_000_000_000, notes: "draft" }];
    const { container } = await openNew(fake, "login");
    await expectNoAxeViolations(container);
    for (const type of ["address", "card", "notes", "files"] as const) {
      navigation.closeEditor();
      navigation.openEditor(await api.entryNewModel(type, null));
      await screen.findByRole("textbox", { name: "Title" });
      await expectNoAxeViolations(container);
    }
  });

  it("editor_with_a_draft_banner_passes_axe", async () => {
    const fake = backendWith([{ id: "e-note", title: "Memo", entryType: "notes", notes: "x", modified: 1 }]);
    fake.state.drafts["e-note"] = [{ id: "d1", savedAt: 1_700_000_000_000, notes: "draft" }];
    const { container } = await openEdit(fake, "e-note");
    await screen.findByRole("region", { name: "Draft found" });
    await expectNoAxeViolations(container);
  });
});

describe("list actions on the entry open in the editor", () => {
  async function editing(): Promise<ReturnType<typeof backendWith>> {
    const fake = backendWith([{ id: "e-bank", title: "Bank", password: "pw", modified: 1 }]);
    await openEdit(fake, "e-bank");
    return fake;
  }
  const row = () => within(screen.getByRole("listbox")).getByRole("option", { name: /Bank/ });

  it("move_restore_and_delete_are_disabled_in_the_menu_for_the_edited_entry", async () => {
    const fake = await editing();
    await fireEvent.contextMenu(row());
    for (const name of ["Move to…", "Delete"]) {
      expect(screen.getByRole("menuitem", { name }).getAttribute("aria-disabled"), name).toBe("true");
    }
    // Another entry's menu is unaffected.
    await fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
    await fireEvent.contextMenu(within(screen.getByRole("listbox")).getByRole("option", { name: /GitHub/ }));
    expect(screen.getByRole("menuitem", { name: "Delete" }).getAttribute("aria-disabled")).toBeNull();
    expect(calls(fake, "entryDelete")).toHaveLength(0);
  });

  it("the_delete_key_and_dragging_do_nothing_for_the_edited_entry", async () => {
    const fake = await editing();
    const bank = row();
    expect(bank.getAttribute("draggable")).toBe("false");
    (screen.getByRole("listbox") as HTMLElement).focus();
    await user().click(bank);
    await user().keyboard("{Delete}");
    expect(calls(fake, "entryDelete")).toHaveLength(0);
    expect(screen.queryByRole("dialog", { name: /Delete/ })).toBeNull();
    expect(navigation.editModel?.id).toBe("e-bank");
  });

  it("after_the_editor_closes_the_entry_can_be_deleted_again", async () => {
    const fake = await editing();
    await user().click(screen.getByRole("button", { name: "Cancel" }));
    await fireEvent.contextMenu(row());
    await user().click(screen.getByRole("menuitem", { name: "Delete" }));
    await waitFor(() => expect(calls(fake, "entryDelete")).toHaveLength(1));
  });
});
