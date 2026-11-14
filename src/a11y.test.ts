// Whole-window accessibility: axe over the real roots, accessible names of every dialog, focus
// return after every modal, and the tab order across the main window's panes.
import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { expectNoAxeViolations } from "./test/axe";
import { createFakeBackend, type FakeBackend } from "./test/fakeBackend";
import { dragAndDrop } from "./test/dnd";
import { makeEntry } from "./test/fixtures";
import { renderMainApp } from "./test/mainApp";
import { renderSettingsApp } from "./test/settingsApp";
import { sourceFiles } from "./test/sourceScan";
import { renderTrayPanel } from "./test/trayApp";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
import { open } from "@tauri-apps/plugin-dialog";

type User = ReturnType<typeof userEvent.setup>;

async function unlocked(prepare?: (fake: FakeBackend) => void) {
  const fake = createFakeBackend({ session: "unlocked" });
  prepare?.(fake);
  const app = await renderMainApp(fake);
  await screen.findByTestId("entry-list");
  await waitFor(() => expect(screen.queryAllByRole("option").length).toBeGreaterThan(0));
  return { ...app, fake, user: userEvent.setup() };
}

const pane = () => screen.getByRole("region", { name: "Details" });

describe("axe on the real windows", () => {
  it("no_axe_violations_on_main_screens", async () => {
    // Unlock, with a vault and without one.
    const locked = await renderMainApp({ session: "locked" });
    await screen.findByLabelText("Master password");
    await expectNoAxeViolations(locked.container);
    locked.unmount();
    const none = await renderMainApp({ session: "noVault" });
    await screen.findByRole("button", { name: "Create vault" });
    await expectNoAxeViolations(none.container);
    none.unmount();

    // The shell with a selected login, then its editor.
    const { user, container } = await unlocked();
    await user.click(screen.getByRole("option", { name: /GitHub/ }));
    await screen.findByRole("heading", { name: "GitHub" });
    await expectNoAxeViolations(container);
    await user.click(within(pane()).getByRole("button", { name: "Edit" }));
    await screen.findByRole("button", { name: "Save" });
    await expectNoAxeViolations(container);
    await user.click(screen.getByRole("button", { name: "Cancel" }));
  });

  it("generator_and_history_pass_axe", async () => {
    const { user, fake } = await unlocked((f) =>
      f.state.entries.push(
        makeEntry({ id: "e-r", title: "Router", modified: 5, history: [{ modified: 1, title: "Router 1", password: "x" }] }),
      ),
    );
    await user.click(screen.getByRole("button", { name: "New Entry" }));
    await user.click(await screen.findByRole("menuitem", { name: "Password Generator…" }));
    const generator = await screen.findByRole("dialog", { name: "Password Generator…" });
    await user.click(within(generator).getByRole("button", { name: "Generate" }));
    await expectNoAxeViolations(generator);
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("option", { name: /Router/ }));
    await user.click(await screen.findByRole("button", { name: /^View history/ }));
    const history = await screen.findByRole("dialog", { name: "History" });
    await within(history).findByRole("listbox");
    await expectNoAxeViolations(history);
    expect(fake.calls.length).toBeGreaterThan(0);
  });

  it("every_settings_pane_passes_axe", async () => {
    for (const pane of ["General", "Vault", "Security", "Browser", "Appearance", "About"]) {
      const app = await renderSettingsApp({}, { pane });
      await screen.findByRole("tablist");
      await expectNoAxeViolations(app.container);
      app.unmount();
    }
  });

  it("the_tray_panel_passes_axe_locked_unlocked_and_in_detail", async () => {
    const locked = await renderTrayPanel({ session: "locked" });
    await screen.findByLabelText("Master password");
    await expectNoAxeViolations(locked.container);
    locked.unmount();
    const app = await renderTrayPanel({ session: "unlocked" });
    const user = userEvent.setup();
    await expectNoAxeViolations(app.container);
    await user.click(await screen.findByRole("button", { name: "GitHub" }));
    await screen.findByRole("region", { name: "GitHub" });
    await expectNoAxeViolations(app.container);
  });
});

/** The top dialog, once focus has moved into it (the modal does that a tick after it mounts). */
async function topDialog(name?: string | RegExp): Promise<HTMLElement> {
  const dialog = await screen.findByRole("dialog", name === undefined ? undefined : { name });
  await waitFor(() => expect(dialog.contains(document.activeElement), `${dialog.getAttribute("aria-labelledby")} ${nameOf(dialog)} active=${document.activeElement?.outerHTML.slice(0, 120)}`).toBe(true));
  return dialog;
}

// The names a dialog can take: aria-label, or aria-labelledby pointing at text that is there.
function nameOf(dialog: HTMLElement): string {
  const label = dialog.getAttribute("aria-label");
  if (label) return label.trim();
  const ids = (dialog.getAttribute("aria-labelledby") ?? "").split(/\s+/).filter(Boolean);
  return ids.map((id) => document.getElementById(id)?.textContent ?? "").join(" ").trim();
}

describe("dialogs", () => {
  it("every_modal_is_given_a_title_and_the_component_names_the_dialog_with_it", () => {
    const users = sourceFiles(["svelte"]).filter((f) => f.path !== "src/lib/ui/Modal.svelte" && /<Modal\b/.test(f.text));
    expect(users.length).toBeGreaterThan(0);
    const missing: string[] = [];
    for (const f of users) {
      for (const m of f.text.matchAll(/<Modal\b[\s\S]*?>/g)) {
        if (!/\btitle=/.test(m[0])) missing.push(f.path);
      }
    }
    expect(missing).toEqual([]);
    const modal = sourceFiles(["svelte"]).find((f) => f.path === "src/lib/ui/Modal.svelte")?.text ?? "";
    expect(modal).toMatch(/aria-labelledby/);
  });

  it("all_dialogs_have_accessible_names", async () => {
    // Open a spread of dialogs for real and read their computed names.
    const { user } = await unlocked((f) => {
      f.state.csvPreview = { total: 1, sampleTitles: ["Acme"], skipped: [] };
    });
    await user.click(screen.getByRole("button", { name: "New Entry" }));
    await user.click(await screen.findByRole("menuitem", { name: "New Group" }));
    let dialog = await topDialog();
    expect(nameOf(dialog)).toBe("New Group");
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: "Import" }));
    await user.click(await screen.findByRole("menuitem", { name: "Import CSV…" }));
    dialog = await topDialog();
    expect(nameOf(dialog)).toBe("Import CSV");
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("option", { name: /GitHub/ }));
    await user.click(within(pane()).getByRole("button", { name: "Edit" }));
    await user.type(await screen.findByLabelText("Title"), "x");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    dialog = await topDialog();
    expect(nameOf(dialog)).toBe("Discard changes?");
    for (const d of screen.getAllByRole("dialog")) expect(nameOf(d)).not.toBe("");
  });
});

// Focus return: every dialog gives focus back to what opened it. Each scenario opens one modal
// the way a user does and answers with the element that must hold focus once Escape closes it.
// The set of files is discovered from the sources, so a new `<Modal` without a scenario fails here.
type Scenario = (user: User) => Promise<{ dialog: string | RegExp; opener: HTMLElement | (() => HTMLElement) }>;

const click = (user: User, el: HTMLElement) => user.click(el);
const menuTrigger = (name: string) => screen.getByRole("button", { name });

async function chooseFromMenu(user: User, button: string, item: string) {
  const opener = menuTrigger(button);
  await user.click(opener);
  await user.click(await screen.findByRole("menuitem", { name: item }));
  return opener;
}

async function selectEntry(user: User, name: RegExp) {
  await user.click(screen.getByRole("option", { name }));
  await screen.findByRole("region", { name: "Details" });
}

const SCENARIOS: Record<string, Scenario> = {
  "src/screens/Toolbar.svelte#Password Generator…": async (user) => {
    await unlocked();
    return { dialog: "Password Generator…", opener: await chooseFromMenu(user, "New Entry", "Password Generator…") };
  },
  "src/screens/GroupEditor.svelte#Edit group…|New Group": async (user) => {
    await unlocked();
    return { dialog: "New Group", opener: await chooseFromMenu(user, "New Entry", "New Group") };
  },
  "src/screens/ImportDialog.svelte#Import CSV": async (user) => {
    await unlocked((f) => (f.state.csvPreview = { total: 1, sampleTitles: ["Acme"], skipped: [] }));
    return { dialog: "Import CSV", opener: await chooseFromMenu(user, "Import", "Import CSV…") };
  },
  "src/screens/OpenVaultSheet.svelte#Open vault": async (user) => {
    vi.mocked(open).mockResolvedValue("/vaults/B.kdbx");
    await renderMainApp({ session: "noVault" });
    const opener = await screen.findByRole("button", { name: "Open existing vault" });
    await click(user, opener);
    return { dialog: "Open vault", opener };
  },
  "src/screens/OpenWarningDialog.svelte#copy.title": async (user) => {
    vi.mocked(open).mockResolvedValue("/vaults/B.kdbx");
    await renderMainApp({ session: "noVault" }, (f) => (f.state.openWarnings = [{ kind: "icloud" }]));
    const opener = await screen.findByRole("button", { name: "Open existing vault" });
    await click(user, opener);
    return { dialog: /./, opener };
  },
  "src/screens/CreateVault.svelte#Create vault": async (user) => {
    await renderMainApp({ session: "noVault" });
    const opener = await screen.findByRole("button", { name: "Create vault" });
    await click(user, opener);
    return { dialog: "Create vault", opener };
  },
  "src/screens/FindVaults.svelte#Find vault files": async (user) => {
    await renderMainApp({ session: "noVault" });
    const opener = await screen.findByRole("button", { name: "Find vault files" });
    await click(user, opener);
    return { dialog: "Find vault files", opener };
  },
  "src/screens/EntryEditor.svelte#Discard changes?": async (user) => {
    await unlocked();
    await selectEntry(user, /GitHub/);
    await user.click(within(pane()).getByRole("button", { name: "Edit" }));
    await user.type(await screen.findByLabelText("Title"), "x");
    const opener = screen.getByRole("button", { name: "Cancel" });
    await click(user, opener);
    return { dialog: "Discard changes?", opener };
  },
  "src/screens/editors/WebsitesSheet.svelte#Websites": async (user) => {
    await unlocked();
    await selectEntry(user, /GitHub/);
    await user.click(within(pane()).getByRole("button", { name: "Edit" }));
    const opener = await screen.findByRole("button", { name: /^\d+ websites?$/ });
    await click(user, opener);
    return { dialog: "Websites", opener };
  },
  "src/screens/editors/NotesForm.svelte#Drafts": async (user) => {
    await unlocked((f) => (f.state.drafts["e-notes"] = [{ id: "d1", savedAt: 1_700_000_000_000, notes: "draft" }]));
    await selectEntry(user, /Server notes/);
    await user.click(within(pane()).getByRole("button", { name: "Edit" }));
    const opener = await screen.findByRole("button", { name: "Drafts" });
    await click(user, opener);
    return { dialog: "Drafts", opener };
  },
  "src/screens/History.svelte#History": async (user) => {
    await unlocked((f) =>
      f.state.entries.push(makeEntry({ id: "e-r", title: "Router", modified: 5, history: [{ modified: 1, title: "R1", password: "x" }] })),
    );
    await selectEntry(user, /Router/);
    const opener = await screen.findByRole("button", { name: /^View history/ });
    await click(user, opener);
    return { dialog: "History", opener };
  },
  "src/screens/EntryDetail.svelte#Delete permanently": async (user) => {
    await unlocked();
    await user.click(screen.getByRole("treeitem", { name: /^Deleted/ }));
    await screen.findByRole("option", { name: /Old forum/ });
    await selectEntry(user, /Old forum/);
    const opener = await within(pane()).findByRole("button", { name: "Delete permanently" });
    await click(user, opener);
    return { dialog: "Delete permanently", opener };
  },
  "src/screens/EntryList.svelte#Empty Deleted|Delete permanently": async (user) => {
    await unlocked();
    await user.click(screen.getByRole("treeitem", { name: /^Deleted/ }));
    await screen.findByRole("option", { name: /Old forum/ });
    const opener = await screen.findByRole("button", { name: "Empty Deleted" });
    await click(user, opener);
    return { dialog: "Empty Deleted", opener };
  },
  "src/screens/GroupPicker.svelte#Move to Group": async (user) => {
    await unlocked();
    await selectEntry(user, /GitHub/);
    // The picker opens from the list's context menu; the listbox owns focus (active descendant).
    const opener = screen.getByRole("listbox");
    await fireEvent.contextMenu(screen.getByRole("option", { name: /GitHub/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Move to…" }));
    return { dialog: "Move to Group", opener };
  },
  "src/screens/Sidebar.svelte#Delete group": async (user) => {
    await unlocked();
    await fireEvent.contextMenu(screen.getByRole("treeitem", { name: /^Work/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Delete group" }));
    return { dialog: "Delete group", opener: screen.getByRole("treeitem", { name: /^Work/ }) };
  },
  "src/screens/History.svelte#Restore this version": async (user) => {
    await unlocked((f) =>
      f.state.entries.push(makeEntry({ id: "e-r", title: "Router", modified: 5, history: [{ modified: 1, title: "R1", password: "x" }] })),
    );
    await selectEntry(user, /Router/);
    await user.click(await screen.findByRole("button", { name: /^View history/ }));
    const history = await topDialog("History");
    await user.click(await within(history).findByRole("option"));
    await click(user, within(history).getByRole("button", { name: "Restore this version" }));
    // The History list re-renders under the confirmation, so the button is looked up again.
    return { dialog: "Restore this version", opener: () => within(history).getByRole("button", { name: "Restore this version" }) };
  },
  "src/screens/Sidebar.svelte#Delete permanently": async (user) => {
    // With the recycle bin off, dropping an entry on Deleted asks before purging it.
    await unlocked((f) => (f.state.recycleBinEnabled = false));
    const row = screen.getByRole("option", { name: /GitHub/ });
    await user.click(row);
    const opener = screen.getByRole("listbox");
    await dragAndDrop(row, screen.getByRole("treeitem", { name: /^Deleted/ }));
    return { dialog: "Delete permanently", opener };
  },
  // The opener contract here is the row's Reveal/Hide button: the context menu was opened on the
  // row and closes back to it, so that is where the dialog returns focus.
  "src/screens/LargeType.svelte#Large Type": async (user) => {
    await unlocked();
    await selectEntry(user, /GitHub/);
    const row = within(pane()).getByRole("group", { name: "Password" });
    await user.click(within(row).getByRole("button", { name: "Reveal" }));
    await fireEvent.contextMenu(row);
    await user.click(await screen.findByRole("menuitem", { name: "Show Large Type" }));
    return { dialog: "Large Type", opener: within(row).getByRole("button", { name: /Hide|Reveal/ }) };
  },
  // A sheet the backend raises: focus comes back to whatever held it before the code appeared.
  "src/screens/PairingCodeSheet.svelte#Pairing code": async () => {
    const { fake } = await unlocked();
    const opener = screen.getByRole("listbox");
    opener.focus();
    const challenge = { code: "123456", browser: "Chrome", expiresAt: Date.now() + 60_000 };
    fake.state.browser.pairing = challenge;
    await fake.emit("pairingChallengeEvent", challenge);
    return { dialog: "Pairing code", opener };
  },
  "src/windows/settings/BrowserHelpSheet.svelte#Connect a browser": async (user) => {
    await renderSettingsApp({}, { pane: "Browser" });
    const opener = await screen.findByRole("button", { name: "Connect a browser" });
    await click(user, opener);
    return { dialog: "Connect a browser", opener };
  },
  "src/windows/settings/Browser.svelte#Revoke this browser?": async (user) => {
    await renderSettingsApp(
      {},
      { pane: "Browser", prepare: (f) => (f.state.browser.paired = [{ id: "b1", label: "Chrome", createdAt: 1_700_000_000_000, diagnostics: null }]) },
    );
    const opener = await screen.findByRole("button", { name: "Revoke: Chrome" });
    await click(user, opener);
    return { dialog: "Revoke this browser?", opener };
  },
  "src/windows/settings/Browser.svelte#Reset paired browsers?": async (user) => {
    await renderSettingsApp({}, { pane: "Browser", prepare: (f) => (f.state.browser.pairingStoreFailed = true) });
    const opener = await screen.findByRole("button", { name: "Reset paired browsers" });
    await click(user, opener);
    return { dialog: "Reset paired browsers?", opener };
  },
  "src/windows/settings/Vault.svelte#Restore this backup?": async (user) => {
    await renderSettingsApp(
      {},
      { pane: "Vault", prepare: (f) => (f.state.backups = [{ id: "backup-1700000000.kdbx", created: 1_700_000_000_000, size: 4096 }]) },
    );
    const opener = await screen.findByRole("button", { name: "Restore…" });
    await click(user, opener);
    return { dialog: "Restore this backup?", opener };
  },
};

/** The balanced `{...}` starting at `from` (string literals skipped). */
function braced(src: string, from: number): number {
  let depth = 0;
  let quote: string | null = null;
  for (let i = from; i < src.length; i++) {
    const c = src.charAt(i);
    if (quote) {
      if (c === "\\") i++;
      else if (c === quote) quote = null;
    } else if (c === '"' || c === "'" || c === "`") quote = c;
    else if (c === "{") depth++;
    else if (c === "}" && --depth === 0) return i + 1;
  }
  return src.length;
}

/** `path#title` of every `<Modal` in the sources; a title built from `t("...")` calls lists them all, `|` apart. */
function modalKeys(): string[] {
  const keys: string[] = [];
  for (const f of sourceFiles(["svelte"])) {
    if (f.path === "src/lib/ui/Modal.svelte") continue;
    for (const m of f.text.matchAll(/<Modal\b/g)) {
      let i = m.index + 6;
      let titleExpr = "";
      while (i < f.text.length && f.text.charAt(i) !== ">") {
        if (f.text.charAt(i) === "{") {
          const end = braced(f.text, i);
          if (f.text.slice(m.index, i).trimEnd().endsWith("title=")) titleExpr = f.text.slice(i + 1, end - 1);
          i = end;
        } else i++;
      }
      const literals = [...titleExpr.matchAll(/\bt\(\s*"([^"]*)"/g)].map((l) => l[1]);
      keys.push(`${f.path}#${literals.length > 0 ? literals.join("|") : titleExpr.trim()}`);
    }
  }
  return keys.sort();
}

describe("focus return", () => {
  const modals = modalKeys();

  it("every_modal_in_the_sources_has_a_focus_scenario_and_no_scenario_is_stale", () => {
    // Two modals with the same title in one file would share one scenario unnoticed.
    expect(new Set(modals).size).toBe(modals.length);
    expect(modals.length).toBeGreaterThan(15);
    expect(modals.filter((k) => !(k in SCENARIOS))).toEqual([]);
    expect(Object.keys(SCENARIOS).filter((k) => !modals.includes(k))).toEqual([]);
  });

  it.each(Object.keys(SCENARIOS))("focus_returns_to_trigger_after_modal_closes: %s", async (file) => {
    const user = userEvent.setup();
    const { dialog, opener } = await (SCENARIOS[file] as Scenario)(user);
    const found = await topDialog(dialog);
    expect(found).toBeTruthy();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: dialog })).toBeNull());
    expect(document.activeElement).toBe(typeof opener === "function" ? opener() : opener);
  });
});

describe("keyboard", () => {
  it("tab_order_reaches_toolbar_sidebar_list_detail_in_order", async () => {
    const { user } = await unlocked();
    await user.click(screen.getByRole("option", { name: /GitHub/ }));
    await screen.findByRole("heading", { name: "GitHub" });
    (document.activeElement as HTMLElement | null)?.blur();
    // The toolbar is the header that holds the search field (the list and the detail have headers too).
    const toolbar = screen.getByRole("searchbox").closest("header");
    const zoneOf = (el: Element | null): string | null => {
      if (!el) return null;
      if (toolbar?.contains(el)) return "toolbar";
      if (el.closest("[role=tree]")) return "sidebar";
      if (el.closest("[role=listbox]")) return "list";
      if (el.closest("[role=region]")) return "detail";
      return null;
    };
    const seen: string[] = [];
    for (let i = 0; i < 80; i += 1) {
      await user.tab();
      const zone = zoneOf(document.activeElement);
      if (zone && seen[seen.length - 1] !== zone) seen.push(zone);
      if (seen.includes("detail")) break;
    }
    expect(seen).toEqual(["toolbar", "sidebar", "list", "detail"]);
  });
});
