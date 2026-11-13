import { screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import type { BrowserDiagnosticsDto_Serialize, BrowserStatus_Serialize } from "../../lib/bindings";
import { setLanguage } from "../../lib/i18n/index.svelte";
import { browser } from "../../lib/stores/browser.svelte";
import { expectNoAxeViolations } from "../../test/axe";
import { calls } from "../../test/mainApp";
import { renderSettingsApp } from "../../test/settingsApp";
import { sourceFiles } from "../../test/sourceScan";
import { untranslatedFindings } from "../../test/textScan";
import en from "../../lib/i18n/locales/en.json";
import ru from "../../lib/i18n/locales/ru.json";
import uk from "../../lib/i18n/locales/uk.json";
import type { FakeBackend } from "../../test/fakeBackend";

afterEach(() => setLanguage("en"));

const CONNECTED = Date.UTC(2026, 10, 1, 12, 0);

const diagnostics = (over: Partial<BrowserDiagnosticsDto_Serialize> = {}): BrowserDiagnosticsDto_Serialize => ({
  proxyState: "active",
  permissionState: "granted",
  passwordSavingEnabled: false,
  extensionScanState: "complete",
  enabledManagers: [],
  receivedAt: CONNECTED,
  ...over,
});

const status = (over: Partial<BrowserStatus_Serialize> = {}): BrowserStatus_Serialize => ({
  listening: true,
  pairingStoreFailed: false,
  paired: [{ id: "b1", label: "Chrome (Work)", createdAt: CONNECTED, diagnostics: null }],
  pairing: null,
  registrations: [],
  hostPath: null,
  ...over,
});

const open = (over: Partial<BrowserStatus_Serialize> = {}) =>
  renderSettingsApp({}, { pane: "Browser", prepare: (fake) => void (fake.state.browser = status(over)) });

const panel = () => screen.getByRole("tabpanel");

describe("Browser pane", () => {
  it("browser_pane_lists_paired_browsers_with_connected_date", async () => {
    await open();
    const row = (await within(panel()).findByText("Chrome (Work)")).closest("li") as HTMLElement;
    expect(within(row).getByText(/^Connected .*2026/)).toBeTruthy();
    expect(within(panel()).getByText("Listening")).toBeTruthy();
  });

  it("an_empty_list_and_a_stopped_bridge_say_so", async () => {
    await open({ listening: false, paired: [] });
    expect(await within(panel()).findByText("Not running")).toBeTruthy();
    expect(within(panel()).getByText("No browsers connected yet")).toBeTruthy();
  });

  it("revoke_requires_confirmation_then_calls_browser_revoke", async () => {
    const user = userEvent.setup();
    const { fake } = await open();
    await user.click(await screen.findByRole("button", { name: "Revoke: Chrome (Work)" }));
    const dialog = await screen.findByRole("dialog", { name: "Revoke this browser?" });
    expect(calls(fake, "browserRevoke")).toHaveLength(0);
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(calls(fake, "browserRevoke")).toHaveLength(0);
    await user.click(screen.getByRole("button", { name: "Revoke: Chrome (Work)" }));
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Revoke" }));
    await waitFor(() => expect(calls(fake, "browserRevoke").map((c) => c.args.id)).toEqual(["b1"]));
    await waitFor(() => expect(within(panel()).queryByText("Chrome (Work)")).toBeNull());
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("status_refreshes_on_bridge_changed_event", async () => {
    const { fake } = await open();
    await within(panel()).findByText("Chrome (Work)");
    fake.state.browser = status({ paired: [{ id: "b2", label: "Edge", createdAt: CONNECTED, diagnostics: null }] });
    await fake.emit("bridgeChangedEvent", null);
    expect(await within(panel()).findByText("Edge")).toBeTruthy();
    expect(within(panel()).queryByText("Chrome (Work)")).toBeNull();
  });

  it("diagnostics_lines_use_localised_catalog_strings_for_each_proxy_state", async () => {
    const expected: Record<string, string> = {
      active: "MyPass was ready to receive passkey requests at the last check.",
      disabled: "MyPass passkeys were turned off at the last check.",
      locked: "Unlock MyPass, then reopen its browser extension.",
      unpaired: "Connect this browser to MyPass again.",
      unavailable: "Open MyPass and reopen its browser extension to check passkeys.",
      "attachment-error": "MyPass could not handle passkeys. Disable other passkey extensions, then check again.",
    };
    expect(Object.keys(expected)).toHaveLength(6);
    const { fake } = await open();
    for (const [proxyState, line] of Object.entries(expected)) {
      fake.state.browser = status({
        paired: [{ id: "b1", label: "Chrome", createdAt: CONNECTED, diagnostics: diagnostics({ proxyState }) }],
      });
      await fake.emit("bridgeChangedEvent", null);
      expect(await within(panel()).findByText(line)).toBeTruthy();
    }
    // The same lines read in the other languages.
    setLanguage("uk");
    expect(await within(panel()).findByText(uk["MyPass could not handle passkeys. Disable other passkey extensions, then check again."] as string)).toBeTruthy();
    setLanguage("ru");
    expect(await within(panel()).findByText(ru["MyPass could not handle passkeys. Disable other passkey extensions, then check again."] as string)).toBeTruthy();
  });

  it("diagnostics_cover_saving_and_manager_lines", async () => {
    await open({
      paired: [
        {
          id: "b1",
          label: "Chrome",
          createdAt: CONNECTED,
          diagnostics: diagnostics({ passwordSavingEnabled: true, enabledManagers: ["Bitwarden", "1Password"] }),
        },
      ],
    });
    expect(await within(panel()).findByText("Browser password saving: on")).toBeTruthy();
    expect(within(panel()).getByText("Enabled managers: Bitwarden, 1Password. Disable them in this browser profile.")).toBeTruthy();
  });

  it("native_host_rows_show_state_chip_and_repair_calls_reinstall", async () => {
    const user = userEvent.setup();
    const { fake } = await open({
      registrations: [
        { id: "chrome", display: "Google Chrome", state: "ok", manifestPath: null, detail: null },
        { id: "edge", display: "Microsoft Edge", state: "missing", manifestPath: null, detail: null },
        { id: "brave", display: "Brave", state: "stale", manifestPath: null, detail: null },
        { id: "arc", display: "Arc", state: "error", manifestPath: null, detail: "x" },
      ],
    });
    await within(panel()).findByText("Google Chrome");
    expect([...panel().querySelectorAll("[data-testid=registration-state]")].map((c) => c.textContent)).toEqual([
      "OK",
      "Missing",
      "Out of date",
      "Error",
    ]);
    fake.state.browserReinstall = [{ id: "edge", display: "Microsoft Edge", state: "ok", manifestPath: null, detail: null }];
    await user.click(screen.getByRole("button", { name: "Repair" }));
    await waitFor(() => expect(calls(fake, "browserReinstallHost")).toHaveLength(1));
    expect(await within(panel()).findByText("Microsoft Edge")).toBeTruthy();
    expect(within(panel()).queryByText("Brave")).toBeNull();
  });

  it("pairing_store_failure_offers_a_confirmed_reset", async () => {
    const user = userEvent.setup();
    const { fake } = await open({ listening: false, pairingStoreFailed: true });
    expect(await screen.findByText("The list of connected browsers could not be read, so browser integration is not running.")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Reset paired browsers" }));
    const dialog = await screen.findByRole("dialog", { name: "Reset paired browsers?" });
    expect(calls(fake, "browserResetPairings")).toHaveLength(0);
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(calls(fake, "browserResetPairings")).toHaveLength(0);
    await user.click(screen.getByRole("button", { name: "Reset paired browsers" }));
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Reset" }));
    await waitFor(() => expect(calls(fake, "browserResetPairings")).toHaveLength(1));
    await waitFor(() => expect(screen.queryByText(/could not be read/)).toBeNull());
    expect(within(panel()).getByText("Listening")).toBeTruthy();
  });

  it("the_stored_status_never_keeps_the_pairing_code", async () => {
    await open({ pairing: { code: "654321", browser: "Chrome", expiresAt: Date.now() + 60_000 } });
    await within(panel()).findByText("Chrome (Work)");
    expect(JSON.stringify(browser.status)).not.toContain("654321");
    expect(document.body.textContent).not.toContain("654321");
  });

  it("show_extension_folder_button_calls_command", async () => {
    const user = userEvent.setup();
    const { fake } = await open();
    await user.click(await screen.findByRole("button", { name: "Show extension folder" }));
    expect(calls(fake, "browserShowExtensionFolder")).toHaveLength(1);
    await user.click(screen.getByRole("button", { name: "Connect a browser" }));
    const dialog = await screen.findByRole("dialog", { name: "Connect a browser" });
    await user.click(within(dialog).getByRole("button", { name: "Show extension folder" }));
    expect(calls(fake, "browserShowExtensionFolder")).toHaveLength(2);
  });

  it("browser_pane_has_no_untranslated_text_nodes", () => {
    const files = sourceFiles(["svelte"]).filter((f) =>
      /(settings\/Browser|settings\/BrowserHelpSheet|PairingCodeSheet|ProposalHandler)\.svelte$/.test(f.path),
    );
    expect(files).toHaveLength(4);
    for (const f of files) expect(untranslatedFindings(f.text), f.path).toEqual([]);
  });

  it("browser_pane_passes_axe", async () => {
    await open({
      paired: [{ id: "b1", label: "Chrome", createdAt: CONNECTED, diagnostics: diagnostics() }],
      registrations: [{ id: "chrome", display: "Google Chrome", state: "ok", manifestPath: null, detail: null }],
    });
    await within(panel()).findByText("Chrome");
    await expectNoAxeViolations(panel());
  });

  it("selects_the_browser_pane_on_a_settings_pane_event", async () => {
    const { fake } = await renderSettingsApp({}, { prepare: (f: FakeBackend) => void (f.state.browser = status()) });
    expect(screen.getByRole("tab", { name: "General", selected: true })).toBeTruthy();
    await fake.emit("settingsPaneEvent", { pane: "browser" });
    await waitFor(() => expect(screen.getByRole("tab", { name: "Browser", selected: true })).toBeTruthy());
  });
});

describe("settings hash", () => {
  it("a window opened at the settings browser hash starts on the Browser pane", async () => {
    window.location.hash = "#/settings/browser";
    try {
      await renderSettingsApp({}, { pane: "About", prepare: (f: FakeBackend) => void (f.state.browser = status()) });
      expect(screen.getByRole("tab", { name: "Browser", selected: true })).toBeTruthy();
    } finally {
      window.location.hash = "";
    }
  });
});

describe("BrowserHelpSheet", () => {
  const openHelp = async () => {
    const user = userEvent.setup();
    const app = await open();
    await user.click(await screen.findByRole("button", { name: "Connect a browser" }));
    return { ...app, user, dialog: await screen.findByRole("dialog", { name: "Connect a browser" }) };
  };

  it("help_sheet_shows_extension_id_hjijbabodfldconmhdcdphlbgaajkejn_and_three_steps", async () => {
    const { dialog } = await openHelp();
    expect(within(dialog).getByText("hjijbabodfldconmhdcdphlbgaajkejn")).toBeTruthy();
    const steps = within(within(dialog).getByRole("list", { name: "Connect a browser" })).getAllByRole("listitem");
    expect(steps).toHaveLength(3);
    expect(within(dialog).getByText("Browsers must be restarted once after the first launch of MyPass.")).toBeTruthy();
  });

  it("help_sheet_has_other_managers_section_with_chrome_bitwarden_1password_and_proxy_conflict", async () => {
    const { dialog } = await openHelp();
    for (const name of ["Chrome’s own autofill and Google Password Manager", "Bitwarden", "1Password", "iCloud Passwords", "Why passkeys conflict", "Check the result"]) {
      expect(within(dialog).getByRole("heading", { name }), name).toBeTruthy();
    }
    expect(within(dialog).getByText(/webAuthenticationProxy/)).toBeTruthy();
    // Addresses are selectable text, never a clipboard action.
    expect(within(dialog).getByText("chrome://password-manager/settings")).toBeTruthy();
    expect(within(dialog).getByText("chrome://extensions")).toBeTruthy();
    expect(within(dialog).queryByRole("button", { name: "Copy address" })).toBeNull();
    const hosts = within(dialog).getAllByRole("link").map((a) => new URL(a.getAttribute("href") ?? "").host);
    expect(new Set(hosts)).toEqual(new Set(["support.google.com", "bitwarden.com", "support.1password.com", "support.apple.com"]));
  });

  it("help_sheet_manager_keys_have_uk_and_ru", () => {
    const keys = Object.keys(en).filter((k) => /Bitwarden|1Password|Why passkeys|webAuthenticationProxy|Extension ID|extension folder|restarted once|Developer mode|extension popup|6-digit/.test(k));
    expect(keys.length).toBeGreaterThanOrEqual(9);
    for (const k of keys) {
      expect((uk as Record<string, string>)[k], `uk ${k}`).toBeTruthy();
      expect((ru as Record<string, string>)[k], `ru ${k}`).toBeTruthy();
    }
  });
});
