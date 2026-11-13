import { afterEach, describe, expect, it } from "vitest";
import { fixtureEntries } from "../../test/fixtures";
import { navigation } from "./navigation.svelte";

afterEach(() => navigation.reset());

describe("navigation store", () => {
  it("starts_on_all_items_in_browse_mode", () => {
    expect(navigation.scope).toEqual({ kind: "category", category: "all" });
    expect(navigation.selectedId).toBeNull();
    expect(navigation.presentation).toBe("browse");
    expect(navigation.editModel).toBeNull();
  });

  it("set_scope_clears_selection_when_browsing_and_notifies", () => {
    const seen: string[] = [];
    const off = navigation.onScopeChange((s) => seen.push(JSON.stringify(s)));
    navigation.select("e1");
    navigation.setScope({ kind: "group", groupId: "g1" });
    expect(navigation.selectedId).toBeNull();
    expect(seen).toEqual([JSON.stringify({ kind: "group", groupId: "g1" })]);
    off();
    navigation.setScope({ kind: "category", category: "cards" });
    expect(seen).toHaveLength(1);
  });

  it("set_scope_keeps_an_open_editor", () => {
    const model = fixtureEntries()[0]!.model;
    navigation.openEditor(model);
    navigation.setScope({ kind: "group", groupId: "g1" });
    expect(navigation.presentation).toBe("edit");
    expect(navigation.editModel?.title).toBe("GitHub");
  });

  it("open_editor_picks_edit_for_a_saved_entry_and_create_for_a_new_one", () => {
    const model = fixtureEntries()[0]!.model;
    navigation.openEditor(model);
    expect(navigation.presentation).toBe("edit");
    navigation.closeEditor();
    expect(navigation.presentation).toBe("browse");
    expect(navigation.editModel).toBeNull();
    navigation.openEditor({ ...model, id: null });
    expect(navigation.presentation).toBe("create");
  });

  it("reset_returns_everything_to_the_start", () => {
    navigation.select("e1");
    navigation.openEditor(fixtureEntries()[0]!.model);
    navigation.setScope({ kind: "group", groupId: "g1" });
    navigation.reset();
    expect(navigation.scope).toEqual({ kind: "category", category: "all" });
    expect(navigation.selectedId).toBeNull();
    expect(navigation.editModel).toBeNull();
  });
});
