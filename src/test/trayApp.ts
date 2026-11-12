// Renders the real tray panel on a fake backend and starts the stores the way `main.ts` does for
// the `tray-panel` label: no entries list, no "Saving failed" toast (that one belongs to `main`).
import { afterEach } from "vitest";
import { loadPlatform } from "../lib/platform.svelte";
import { startStores } from "../lib/stores";
import { toasts } from "../lib/stores/toasts.svelte";
import TrayPanel from "../windows/TrayPanel.svelte";
import { createFakeBackend, type FakeBackend, type FakeBackendOptions } from "./fakeBackend";
import { renderWithBackend } from "./render";

const stops: (() => void)[] = [];

afterEach(() => {
  for (const stop of stops.splice(0)) stop();
  toasts.clear();
});

export async function renderTrayPanel(
  opts: FakeBackendOptions | FakeBackend = {},
  prepare?: (fake: FakeBackend) => void,
) {
  const fake = "handlers" in opts ? opts : createFakeBackend({ session: "unlocked", ...opts });
  for (const stop of stops.splice(0)) stop();
  prepare?.(fake);
  const utils = renderWithBackend(TrayPanel, {}, fake);
  stops.push(await startStores({ entries: false, saveToast: false }));
  await loadPlatform();
  return { ...utils, fake };
}
