import { afterEach, describe, expect, it, vi } from "vitest";
import type { EntrySummary } from "../bindings";
import { createFakeBackend } from "../../test/fakeBackend";
import { startStores } from "./index";
import { entries } from "./entries.svelte";
import { navigation } from "./navigation.svelte";
import { toasts } from "./toasts.svelte";

let stop: (() => void) | undefined;
afterEach(() => {
  stop?.();
  stop = undefined;
  toasts.clear();
  vi.useRealTimers();
});

async function bootUnlocked() {
  const fake = createFakeBackend({ session: "unlocked" });
  fake.install();
  stop = await startStores();
  await vi.waitFor(() => expect(entries.items.length).toBe(4));
  fake.calls.length = 0;
  return fake;
}
const listCalls = (fake: { calls: { command: string; args: Record<string, any> }[] }) =>
  fake.calls.filter((c) => c.command === "entryList");

describe("entries store", () => {
  it("search_is_debounced_150ms", async () => {
    const fake = await bootUnlocked();
    vi.useFakeTimers();
    entries.setSearch("g");
    await vi.advanceTimersByTimeAsync(50);
    entries.setSearch("gi");
    await vi.advanceTimersByTimeAsync(50);
    entries.setSearch("git");
    await vi.advanceTimersByTimeAsync(149);
    expect(listCalls(fake)).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(1);
    expect(listCalls(fake)).toHaveLength(1);
    expect(listCalls(fake)[0]?.args.query.search).toBe("git");
    expect(entries.searchText).toBe("git");
    expect(entries.items.map((i) => i.title)).toEqual(["GitHub", "GitLab"]);
  });

  it("stale_search_response_ignored", async () => {
    const fake = await bootUnlocked();
    const github = fake.state.entries.find((e) => e.summary.id === "e-github")!.summary;
    const visa = fake.state.entries.find((e) => e.summary.id === "e-visa")!.summary;
    const pending: ((rows: EntrySummary[]) => void)[] = [];
    fake.handlers.entryList = () => new Promise((resolve) => pending.push(resolve));
    const first = entries.setSort("modified");
    const second = entries.setSort("title");
    await vi.waitFor(() => expect(pending).toHaveLength(2));
    pending[1]?.([visa]);
    await second;
    pending[0]?.([github]);
    await first;
    expect(entries.items.map((i) => i.id)).toEqual(["e-visa"]);
    expect(entries.loading).toBe(false);
  });

  it("vault_changed_refetches_current_query_and_sidebar", async () => {
    const fake = await bootUnlocked();
    fake.state.entries.pop();
    await fake.emit("vaultChanged", null);
    await vi.waitFor(() => expect(fake.calls.some((c) => c.command === "sidebarGet")).toBe(true));
    expect(listCalls(fake)).toHaveLength(1);
    expect(listCalls(fake)[0]?.args.query).toEqual({
      scope: { kind: "category", category: "all" },
      search: "",
      sort: "title",
    });
  });

  it("changing_scope_refetches_the_list_only", async () => {
    const fake = await bootUnlocked();
    navigation.setScope({ kind: "category", category: "favorites" });
    await vi.waitFor(() => expect(entries.items.map((i) => i.title)).toEqual(["GitHub"]));
    expect(fake.calls.some((c) => c.command === "sidebarGet")).toBe(false);
    expect(entries.query.scope).toEqual({ kind: "category", category: "favorites" });
  });

  it("a_locked_answer_during_refresh_is_not_toasted", async () => {
    const fake = await bootUnlocked();
    fake.state.session.state = "locked";
    await entries.refresh();
    expect(toasts.items).toHaveLength(0);
  });

  it("other_refresh_errors_are_reported", async () => {
    const fake = await bootUnlocked();
    fake.handlers.entryList = () => {
      throw { code: "io", message: "disk" };
    };
    await entries.refresh();
    expect(toasts.items).toHaveLength(1);
    expect(entries.loading).toBe(false);
  });
});
