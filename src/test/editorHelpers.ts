// Shared by the editor tests: opens the real editor inside the real main window on a fake backend.
import { screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect } from "vitest";
import { api } from "../lib/api";
import type { EntryTypeDto } from "../lib/bindings";
import { navigation } from "../lib/stores/navigation.svelte";
import { createFakeBackend, type FakeBackend } from "./fakeBackend";
import { makeEntry, type EntrySpec } from "./fixtures";
import { renderMainApp } from "./mainApp";

export function backendWith(extra: EntrySpec[] = []): FakeBackend {
  const fake = createFakeBackend({ session: "unlocked" });
  for (const spec of extra) fake.state.entries.push(makeEntry(spec));
  return fake;
}

/** The main window with a new, empty entry of `type` open in the editor. */
export async function openNew(fake: FakeBackend, type: EntryTypeDto, groupId: string | null = null) {
  const app = await renderMainApp(fake);
  await screen.findByRole("listbox");
  navigation.openEditor(await api.entryNewModel(type, groupId));
  await screen.findByRole("button", { name: "Save" });
  return app;
}

/** The main window with the editor of entry `id` open (through the detail pane's Edit button). */
export async function openEdit(fake: FakeBackend, id: string) {
  const app = await renderMainApp(fake);
  await screen.findByRole("listbox");
  navigation.select(id);
  await userEvent.setup().click(await screen.findByRole("button", { name: "Edit" }));
  await screen.findByRole("button", { name: "Save" });
  return app;
}

export const user = () => userEvent.setup();

export async function until(check: () => unknown) {
  await waitFor(() => expect(check()).toBeTruthy());
}
