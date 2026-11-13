// Renders the real settings window on a fake backend and starts the stores the way `main.ts` does
// for the `settings` label (no entries fetch).
import { screen } from "@testing-library/svelte";
import { afterEach } from "vitest";
import { loadPlatform } from "../lib/platform.svelte";
import { startStores } from "../lib/stores";
import { settings } from "../lib/stores/settings.svelte";
import { toasts } from "../lib/stores/toasts.svelte";
import SettingsApp from "../windows/SettingsApp.svelte";
import { createFakeBackend, type FakeBackend, type FakeBackendOptions } from "./fakeBackend";
import { renderWithBackend } from "./render";

const stops: (() => void)[] = [];

afterEach(() => {
  for (const stop of stops.splice(0)) stop();
  toasts.clear();
});

/** Renders the settings window; `pane` is the pane name to open first (stored like a remembered pane). */
export async function renderSettingsApp(
  opts: FakeBackendOptions | FakeBackend = {},
  { pane, prepare }: { pane?: string; prepare?: (fake: FakeBackend) => void } = {},
) {
  const fake = "handlers" in opts ? opts : createFakeBackend({ session: "unlocked", ...opts });
  for (const stop of stops.splice(0)) stop();
  prepare?.(fake);
  try {
    if (pane) window.localStorage.setItem("layout.settingsPane", pane);
    else window.localStorage.removeItem("layout.settingsPane");
  } catch {
    // Storage may be unavailable in a test that breaks it on purpose.
  }
  const utils = renderWithBackend(SettingsApp, {}, fake);
  stops.push(await startStores({ entries: false }));
  await loadPlatform();
  await screen.findByRole("tablist");
  return { ...utils, fake };
}

export { settings };
