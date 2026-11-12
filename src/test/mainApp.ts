// Renders the real main window on a fake backend and starts the stores, the way `main.ts` does.
import { afterEach } from "vitest";
import { loadPlatform } from "../lib/platform.svelte";
import { startStores } from "../lib/stores";
import { toasts } from "../lib/stores/toasts.svelte";
import MainApp from "../windows/MainApp.svelte";
import { createFakeBackend, type CommandName, type FakeBackend, type FakeBackendOptions, type FakeCall } from "./fakeBackend";
import { renderWithBackend } from "./render";

const stops: (() => void)[] = [];

afterEach(() => {
  for (const stop of stops.splice(0)) stop();
  toasts.clear();
});

export async function renderMainApp(opts: FakeBackendOptions | FakeBackend = {}, prepare?: (fake: FakeBackend) => void) {
  const fake = "handlers" in opts ? opts : createFakeBackend(opts);
  // One app at a time: the stores are singletons, so a previous render is stopped first.
  for (const stop of stops.splice(0)) stop();
  prepare?.(fake);
  const utils = renderWithBackend(MainApp, {}, fake);
  stops.push(await startStores());
  await loadPlatform();
  return { ...utils, fake };
}

/** The recorded calls of one command, in order. */
export function calls(fake: { calls: FakeCall[] }, name: CommandName) {
  return fake.calls.filter((c) => c.command === name);
}
