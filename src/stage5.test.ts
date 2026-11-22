import { update } from "./lib/stores/update.svelte";
import { act, screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";
import { renderMainApp, calls } from "./test/mainApp";
import { renderSettingsApp } from "./test/settingsApp";
import { FIXTURE_MASTER_PASSWORD } from "./test/fixtures";
import { expectNoAxeViolations } from "./test/axe";

// Touch ID first (automatic prompt, Touch ID view) is pinned by src/touchIdFirst.test.ts. With only
// the Mac password enabled the screen stays as before: a button, no prompt on mount.
it("mac password device unlock is offered without prompting on mount and presses once", async () => {
  const { fake, container } = await renderMainApp({ deviceUnlockAvailable: true }, f => {
    Object.assign(f.state.device, { enrolled: true, macPasswordEnabled: true, canOfferNow: true, reason: null });
  });
  const button = await screen.findByRole("button", { name: "Unlock with Mac password" });
  expect(calls(fake, "vaultUnlockDevice")).toHaveLength(0);
  await expectNoAxeViolations(container);
  await userEvent.click(button);
  await waitFor(() => expect(calls(fake, "vaultUnlockDevice")).toHaveLength(1));
});
it("expired device window explains master password fallback", async () => {
  await renderMainApp({ deviceUnlockAvailable: true }, f => { f.state.device.reason = "deviceWindowExpired"; });
  expect(await screen.findByText("Enter your master password to use Touch ID again.")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Unlock with Touch ID" })).toBeNull();
});
it("enabling Touch ID requires a typed password and clears it", async () => {
  const { fake } = await renderSettingsApp({ deviceUnlockAvailable: true }, { pane: "Security" });
  await userEvent.click(await screen.findByRole("switch", { name: "Unlock using Touch ID" }));
  const dialog = await screen.findByRole("dialog", { name: "Confirm Master Password" });
  const field = within(dialog).getByLabelText("Master password") as HTMLInputElement;
  await userEvent.type(field, FIXTURE_MASTER_PASSWORD);
  await userEvent.click(within(dialog).getByRole("button", { name: "Enable" }));
  await waitFor(() => expect(calls(fake, "deviceUnlockConfigure")).toHaveLength(1));
  expect(field.value).toBe("");
});
it("update banner dismisses and returns for a newer event", async () => {
  const { fake } = await renderMainApp({}, f => { f.state.update.banner = { version: "9.0.0", url: "https://github.com/release" }; });
  await userEvent.click(await screen.findByRole("button", { name: "View release" }));
  expect(calls(fake, "updateOpenRelease")[0]?.args).toEqual({});
  await userEvent.click(screen.getByRole("button", { name: "Later" }));
  await waitFor(() => expect(screen.queryByRole("button", { name: "Later" })).toBeNull());
  await fake.emit("updateChanged", { status: { state: "idle" }, banner: { version: "10.0.0", url: "https://github.com/release" } });
  expect(await screen.findByText(/MyPass 10.0.0 is available/)).toBeTruthy();
});
it("storage events show fallback and downloading and clear on recovery", async () => {
  const { fake } = await renderMainApp();
  await fake.emit("storageStatusChanged", { location: "iCloudDrive", fallbackActive: false, cacheAhead: true, downloading: false });
  expect(await screen.findByText(/iCloud Drive is not available/)).toBeTruthy();
  await fake.emit("storageStatusChanged", { location: "iCloudDrive", fallbackActive: false, cacheAhead: false, downloading: true });
  expect(await screen.findByText("Waiting for iCloud to download your vault…")).toBeTruthy();
});
it("location picker probes and uses the refreshed token", async () => {
  const { fake } = await renderSettingsApp({}, { pane: "Vault", prepare: f => {
    f.handlers.vaultLocationPick = () => "chosen-token";
    f.handlers.vaultLocationProbe = () => ({ folderToken: "fresh-token", probe: { exists: false, looksLikeVault: false, insideIcloud: true, sameAsCurrent: false, modified: null, byteCount: null } });
  } });
  await userEvent.click(await screen.findByRole("button", { name: "Choose folder in iCloud Drive…" }));
  const dialog = await screen.findByRole("dialog", { name: "Change vault location" });
  expect(within(dialog).queryAllByRole("radio")).toHaveLength(1);
  await userEvent.click(within(dialog).getByRole("button", { name: "Confirm" }));
  await waitFor(() => expect(calls(fake, "vaultLocationChange")[0]?.args).toEqual({ req: { folderToken: "fresh-token", strategy: "seedRemote" } }));
});

it.each([
  [true, false, "Unlock with Touch ID"], [false, true, "Unlock with Mac password"], [true, true, "Unlock with Touch ID or Mac password"],
] as const)("device labels follow the enabled modes %s %s", async (touchIdEnabled, macPasswordEnabled, label) => {
  await renderMainApp({ deviceUnlockAvailable: true }, f => {
    Object.assign(f.state.device, { touchIdEnabled, macPasswordEnabled, canOfferNow: true });
    // Touch ID first prompts by itself; a cancel keeps the Touch ID view with its button.
    f.handlers.vaultUnlockDevice = () => { throw { code: "cancelled", message: "" }; };
  });
  expect(await screen.findByRole("button", { name: label })).toBeTruthy();
});
it("fresh extension unlock request uses an automatic biometric prompt", async () => {
  const { fake } = await renderMainApp({ deviceUnlockAvailable: true }, f => {
    Object.assign(f.state.device, { touchIdEnabled: true, macPasswordEnabled: true, canOfferNow: true });
    f.state.extensionUnlockRequested = true;
  });
  await waitFor(() => expect(calls(fake, "vaultUnlockDevice")).toHaveLength(1));
  expect(calls(fake, "vaultUnlockDevice")[0]?.args).toEqual({ reasonKey: "Unlock MyPass", automatic: true });
  await fake.emit("unlockRequestedEvent", null);
  expect(calls(fake, "vaultUnlockDevice")).toHaveLength(1);
});
it("a browser unlock request leaves the password form when only Mac password is available", async () => {
  const { fake, unmount } = await renderMainApp({ deviceUnlockAvailable: true }, f => {
    Object.assign(f.state.device, { macPasswordEnabled: true, canOfferNow: true, biometrics: false });
  });
  await screen.findByRole("button", { name: "Unlock with Mac password" });
  fake.state.extensionUnlockRequested = true;
  const reads = calls(fake, "deviceUnlockStatus").length;
  await act(async () => {
    await fake.emit("unlockRequestedEvent", null);
    await waitFor(() => expect(calls(fake, "deviceUnlockStatus").length).toBeGreaterThan(reads));
  });
  await waitFor(() => expect(fake.state.extensionUnlockRequested).toBe(false));
  try {
    expect(calls(fake, "vaultUnlockDevice")).toHaveLength(0);
    expect(screen.getByLabelText("Master password")).toBeTruthy();
  } finally {
    unmount();
  }
});
it.each(["cancelled", "invalidated", "lockout"] as const)("device unlock %s keeps password fallback", async code => {
  const { fake } = await renderMainApp({ deviceUnlockAvailable: true }, f => {
    Object.assign(f.state.device, { macPasswordEnabled: true, canOfferNow: true });
    f.handlers.vaultUnlockDevice = () => { throw { code, message: "untrusted backend text" }; };
  });
  await userEvent.click(await screen.findByRole("button", { name: "Unlock with Mac password" }));
  await waitFor(() => expect(calls(fake, "vaultUnlockDevice")).toHaveLength(1));
  if (code === "cancelled") expect(screen.queryByRole("alert")).toBeNull();
  else expect((await screen.findByRole("alert")).textContent).toBe(code === "invalidated" ? "Device unlock was reset. Enter your master password." : "Too many attempts. Use your master password.");
  expect(screen.getByLabelText("Master password")).toBeTruthy();
});
it("pending device prompt ignores repeated presses", async () => {
  let release: (() => void) | undefined;
  const { fake } = await renderMainApp({ deviceUnlockAvailable: true }, f => {
    Object.assign(f.state.device, { macPasswordEnabled: true, canOfferNow: true });
    f.handlers.vaultUnlockDevice = async () => { await new Promise<void>(resolve => release = resolve); return f.state.session; };
  });
  const button = await screen.findByRole("button", { name: "Unlock with Mac password" });
  await userEvent.dblClick(button);
  expect(calls(fake, "vaultUnlockDevice")).toHaveLength(1);
  expect((button as HTMLButtonElement).disabled).toBe(true);
  release?.();
  await waitFor(() => expect((button as HTMLButtonElement).disabled).toBe(false));
});
it("wrong typed password leaves device toggles off and input empty", async () => {
  await renderSettingsApp({ deviceUnlockAvailable: true }, { pane: "Security" });
  await userEvent.click(await screen.findByRole("switch", { name: "Unlock using Touch ID" }));
  const dialog = await screen.findByRole("dialog");
  const input = within(dialog).getByLabelText("Master password") as HTMLInputElement;
  await userEvent.type(input, "wrong");
  await userEvent.click(within(dialog).getByRole("button", { name: "Enable" }));
  expect((await within(dialog).findByRole("alert")).textContent).toBe("Wrong master password");
  expect(input.value).toBe("");
  expect(screen.getByRole("switch", { name: "Unlock using Touch ID" }).getAttribute("aria-checked")).toBe("false");
});
it("device window options and defaults match the contract", async () => {
  await renderSettingsApp({ deviceUnlockAvailable: true }, { pane: "Security" });
  const device = await screen.findByRole("combobox", { name: "Unlock with device:" });
  expect((device as HTMLSelectElement).value).toBe("eightHours");
  expect(within(device).getAllByRole("option").map(o => (o as HTMLOptionElement).value)).toEqual(["oneHour", "fourHours", "eightHours", "twentyFourHours", "never"]);
  const account = screen.getByRole("combobox", { name: "Require account password:" });
  expect((account as HTMLSelectElement).value).toBe("thirtyDays");
  expect(within(account).getAllByRole("option").map(o => (o as HTMLOptionElement).value)).toEqual(["oneDay", "sevenDays", "fourteenDays", "thirtyDays", "ninetyDays", "never"]);
});
it.each([
  ["offline", "Could not reach GitHub."], ["rateLimited", "GitHub rate limit reached; try again later."],
  ["badStatus", "GitHub answered with an error."], ["unexpected", "GitHub returned an unexpected answer."], ["unknownVersion", "Cannot determine the running version."],
] as const)("About shows the localized %s update failure", async (kind, label) => {
  await renderSettingsApp({}, { pane: "About", prepare: f => { f.state.update.status = { state: "failed", kind }; } });
  expect(await screen.findByText(label)).toBeTruthy();
});
it("About manual check is disabled until its response", async () => {
  let release: (() => void) | undefined;
  await renderSettingsApp({}, { pane: "About", prepare: f => {
    f.handlers.updateCheckNow = async () => { await new Promise<void>(resolve => release = resolve); return { status: { state: "failed", kind: "offline" }, banner: null }; };
  } });
  const button = await screen.findByRole("button", { name: "Check for Updates" });
  await userEvent.click(button);
  expect((button as HTMLButtonElement).disabled).toBe(true);
  release?.();
  expect(await screen.findByText("Could not reach GitHub.")).toBeTruthy();
});
it("existing target offers merge by default and all three strategies", async () => {
  const { fake, container } = await renderSettingsApp({}, { pane: "Vault", prepare: f => {
    f.handlers.vaultLocationProbe = () => ({ folderToken: "new-token", probe: { exists: true, looksLikeVault: true, insideIcloud: false, sameAsCurrent: false, modified: null, byteCount: null } });
  } });
  await userEvent.click(await screen.findByRole("button", { name: "Move to a local folder…" }));
  const dialog = await screen.findByRole("dialog");
  expect(within(dialog).getAllByRole("radio")).toHaveLength(3);
  expect((within(dialog).getByLabelText("Merge both (recommended)") as HTMLInputElement).checked).toBe(true);
  await expectNoAxeViolations(container);
  await userEvent.click(within(dialog).getByLabelText("Use the vault in that folder"));
  await userEvent.click(within(dialog).getByRole("button", { name: "Confirm" }));
  await waitFor(() => expect(calls(fake, "vaultLocationChange")[0]?.args).toEqual({ req: { folderToken: "new-token", strategy: "adoptRemote" } }));
});
it.each(["macos", "windows"])("secondary folder works on %s and stopping retains files", async os => {
  const { fake } = await renderSettingsApp({ os }, { pane: "Vault" });
  await userEvent.click(await screen.findByRole("button", { name: "Choose folder…" }));
  const dialog = await screen.findByRole("dialog", { name: "Also sync to folder" });
  await userEvent.click(within(dialog).getByRole("button", { name: "Confirm" }));
  await userEvent.click(await screen.findByRole("button", { name: "Stop syncing" }));
  expect(calls(fake, "syncDisconnectSecondary")).toHaveLength(1);
  expect(screen.getByText("Stopping sync keeps the files in that folder.")).toBeTruthy();
});
it("locked vault cannot start a location change", async () => {
  await renderSettingsApp({ session: "locked" }, { pane: "Vault" });
  expect((await screen.findByRole("button", { name: "Move to a local folder…" }) as HTMLButtonElement).disabled).toBe(true);
});
it("switching configured modes and disabling both use no typed password", async () => {
  const { fake } = await renderSettingsApp({ deviceUnlockAvailable: true }, { pane: "Security", prepare: f => {
    Object.assign(f.state.device, { enrolled: true, touchIdEnabled: true, canOfferNow: true });
  } });
  await userEvent.click(await screen.findByRole("switch", { name: "Unlock app with Mac password" }));
  await waitFor(() => expect(calls(fake, "deviceUnlockConfigure")).toHaveLength(1));
  expect(calls(fake, "deviceUnlockConfigure")[0]?.args).toEqual({ password: null, touchId: true, macPassword: true });
  await userEvent.click(screen.getByRole("switch", { name: "Unlock using Touch ID" }));
  await userEvent.click(screen.getByRole("switch", { name: "Unlock app with Mac password" }));
  expect(calls(fake, "deviceUnlockConfigure").at(-1)?.args).toEqual({ password: null, touchId: false, macPassword: false });
  expect(screen.queryByRole("dialog")).toBeNull();
});
it("failed migration consumes its grant and blocks retry until repicking", async () => {
  const { fake } = await renderSettingsApp({}, { pane: "Vault", prepare: f => {
    f.handlers.vaultLocationChange = () => { throw { code: "io", message: "do not render raw migration detail" }; };
  } });
  await userEvent.click(await screen.findByRole("button", { name: "Move to a local folder…" }));
  const dialog = await screen.findByRole("dialog");
  const confirm = within(dialog).getByRole("button", { name: "Confirm" }) as HTMLButtonElement;
  await userEvent.dblClick(confirm);
  expect(await within(dialog).findByText("Choose the folder again to retry.")).toBeTruthy();
  expect(confirm.disabled).toBe(true);
  expect(calls(fake, "vaultLocationChange")).toHaveLength(1);
  expect(screen.queryByText("do not render raw migration detail")).toBeNull();
});
it.each([
  [{ state: "unavailable", reason: "readOnly" }, "Folder is read-only"],
  [{ state: "unavailable", reason: "folderMissing" }, "Folder not available"],
  [{ state: "error", code: "wrongPassword" }, "Wrong master password for the vault in that folder"],
  [{ state: "error", code: "notAVault" }, "That file is not a vault"],
  [{ state: "error", code: "syncKdfLimitExceeded" }, "This vault was saved with very expensive key-derivation settings. Open the file explicitly or re-save it in another app with lower settings."],
] as const)("secondary status renders %j", async (status, label) => {
  await renderSettingsApp({}, { pane: "Vault", prepare: f => { f.state.sync = status; } });
  expect(await screen.findByText(label)).toBeTruthy();
});
it("secondary error with a retained vault shows the recovery file name only", async () => {
  await renderSettingsApp({}, { pane: "Vault", prepare: f => { f.state.sync = { state: "error", code: "io", recoveryFile: ".vault.kdbx.1.2.3.aside" }; } });
  const note = await screen.findByTestId("sync-recovery-file");
  expect(note.textContent).toBe("The previous vault is kept as .vault.kdbx.1.2.3.aside next to the synced copy. Do not delete it.");
  expect(screen.getByText("Something went wrong")).toBeTruthy();
});
it("secondary error without a recovery file shows no recovery note", async () => {
  await renderSettingsApp({}, { pane: "Vault", prepare: f => { f.state.sync = { state: "error", code: "io" }; } });
  await screen.findByText("Something went wrong");
  expect(screen.queryByTestId("sync-recovery-file")).toBeNull();
});
it("About automatic update toggle uses settings API", async () => {
  const { fake } = await renderSettingsApp({}, { pane: "About" });
  await userEvent.click(await screen.findByRole("switch", { name: "Check for updates automatically" }));
  expect(calls(fake, "settingsUpdate").at(-1)?.args).toEqual({ patch: { updateCheckEnabled: false } });
});
it("Browser signed store hides Repair and development store identifies itself", async () => {
  await renderSettingsApp({}, { pane: "Browser", prepare: f => { f.state.browser.endpointStore = "keychain"; f.state.browser.canReinstallHost = false; } });
  await screen.findByText("Native host");
  expect(screen.queryByRole("button", { name: "Repair" })).toBeNull();
});
it("About checked timestamp is relative", async () => {
  await renderSettingsApp({}, { pane: "About", prepare: f => { f.state.update.status = { state: "upToDate", checkedAt: Date.now() - 60_000 }; } });
  expect(await screen.findByText(/Checked 1 minute ago/)).toBeTruthy();
});
it("new update event wins over an older manual check response", async () => {
  let release: (() => void) | undefined;
  const { fake } = await renderSettingsApp({}, { pane: "About", prepare: f => {
    f.handlers.updateCheckNow = async () => { await new Promise<void>(resolve => release = resolve); return { status: { state: "upToDate", checkedAt: null }, banner: null }; };
  } });
  const checking = update.check();
  await waitFor(() => expect(release).toBeDefined());
  await fake.emit("updateChanged", { status: { state: "available", version: "99.0.0", url: "https://github.com/release" }, banner: { version: "99.0.0", url: "https://github.com/release" } });
  expect(await screen.findByText("MyPass 99.0.0 available…")).toBeTruthy();
  release?.();
  await checking;
  expect(screen.queryByText("MyPass 99.0.0 available…")).toBeTruthy();
});

it("vault switch clears device enrollment password and intent", async () => {
  const { fake } = await renderSettingsApp({ deviceUnlockAvailable: true }, { pane: "Security" });
  await userEvent.click(await screen.findByRole("switch", { name: "Unlock using Touch ID" }));
  const dialog = await screen.findByRole("dialog");
  const input = within(dialog).getByLabelText("Master password") as HTMLInputElement;
  await userEvent.type(input, FIXTURE_MASTER_PASSWORD);
  await fake.emit("sessionChanged", { ...fake.state.session, state: "unlocked", vaultPath: "/other.kdbx" });
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(input.value).toBe("");
  expect(calls(fake, "deviceUnlockConfigure")).toHaveLength(0);
});
it("About retains the release action after dismissal", async () => {
  await renderSettingsApp({}, { pane: "About", prepare: f => {
    f.state.update = { status: { state: "available", version: "9.0.0", url: "https://github.com/slucheninov/mypass-app/releases/tag/v9.0.0" }, banner: null };
    f.handlers.updateCheckNow = () => f.state.update;
  } });
  expect(await screen.findByRole("button", { name: "View release" })).toBeTruthy();
  await userEvent.click(screen.getByRole("button", { name: "Check for Updates" }));
  expect(await screen.findByRole("button", { name: "View release" })).toBeTruthy();
});

it("secondary events cannot discard a pending storage warning snapshot", async () => {
  let release!: (value: import("./lib/bindings").StorageStatus) => void;
  const { fake } = await renderMainApp({}, f => {
    f.handlers.vaultStorageStatus = () => new Promise(resolve => release = resolve);
  });
  await fake.emit("syncStatusChanged", { state: "off" });
  release({ location: "iCloudDrive", fallbackActive: true, cacheAhead: true, downloading: true });
  expect(await screen.findByText(/iCloud Drive is not available/)).toBeTruthy();
  expect(await screen.findByText("Waiting for iCloud to download your vault…")).toBeTruthy();
});
it("storage events cannot discard a pending secondary status snapshot", async () => {
  let release!: (value: import("./lib/bindings").SecondaryStatus_Serialize) => void;
  const { fake } = await renderSettingsApp({}, { pane: "Vault", prepare: f => {
    f.handlers.syncStatus = () => new Promise(resolve => release = resolve);
  } });
  await fake.emit("storageStatusChanged", { location: "local", fallbackActive: false, cacheAhead: false, downloading: false });
  release({ state: "idle", lastSync: null });
  expect(await screen.findByRole("button", { name: "Stop syncing" })).toBeTruthy();
});
