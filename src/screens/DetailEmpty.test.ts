import { screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { navigation } from "../lib/stores/navigation.svelte";
import { expectNoAxeViolations } from "../test/axe";
import { PERSONAL_GROUP_ID, WORK_GROUP_ID, fixtureSidebar } from "../test/fixtures";
import { renderMainApp } from "../test/mainApp";

async function openVault() {
  const app = await renderMainApp({ session: "unlocked" });
  await screen.findByRole("treeitem", { name: /^Personal/ });
  const sidebar = fixtureSidebar(app.fake.state.entries);
  return { ...app, sidebar };
}

const pane = () => screen.getByTestId("detail-empty");
const part = (name: string) => pane().querySelector(`[data-part="${name}"]`)?.textContent?.trim() ?? null;
const countOf = (sidebar: ReturnType<typeof fixtureSidebar>, category: string) =>
  sidebar.categories.find((c) => c.category === category)?.count ?? null;

describe("empty detail pane", () => {
  it("detail_empty_shows_the_vault_and_its_item_count_for_all", async () => {
    const { sidebar } = await openVault();
    expect(navigation.selectedId).toBeNull();
    expect(pane().dataset.kind).toBe("vault");
    expect(part("title")).toBe("Test vault");
    const all = countOf(sidebar, "all") ?? 0;
    expect(part("count")).toBe(`${all} ${all === 1 ? "item" : "items"}`);
    // The key illustration is decorative only.
    const art = pane().querySelector(".art");
    expect(art?.getAttribute("aria-hidden")).toBe("true");
    expect(art?.querySelector("svg")).not.toBeNull();
  });

  it("detail_empty_shows_the_category_icon_label_and_count", async () => {
    const user = userEvent.setup();
    const { sidebar } = await openVault();
    await user.click(screen.getByRole("treeitem", { name: /^Cards/ }));
    await waitFor(() => expect(pane().dataset.kind).toBe("category"));
    expect(part("title")).toBe("Cards");
    const cards = countOf(sidebar, "cards") ?? 0;
    expect(part("count")).toBe(`${cards} ${cards === 1 ? "item" : "items"}`);
  });

  it("detail_empty_security_has_no_count", async () => {
    const user = userEvent.setup();
    await openVault();
    await user.click(screen.getByRole("treeitem", { name: /^Security/ }));
    await waitFor(() => expect(part("title")).toBe("Security"));
    expect(part("count")).toBeNull();
  });

  it("detail_empty_shows_the_group_emoji_or_a_folder_with_name_and_count", async () => {
    const user = userEvent.setup();
    const { sidebar } = await openVault();
    const work = sidebar.groups.find((g) => g.id === WORK_GROUP_ID);
    await user.click(screen.getByRole("treeitem", { name: /Work/ }));
    await waitFor(() => expect(pane().dataset.kind).toBe("group"));
    expect(part("title")).toBe("Work");
    expect(pane().querySelector(".emoji")?.textContent).toBe(work?.emoji);
    expect(part("count")).toContain(String(work?.count));

    const personal = sidebar.groups.find((g) => g.id === PERSONAL_GROUP_ID);
    await user.click(screen.getByRole("treeitem", { name: /^Personal/ }));
    await waitFor(() => expect(part("title")).toBe("Personal"));
    // No emoji: the folder icon stands in.
    expect(pane().querySelector(".emoji")).toBeNull();
    expect(pane().querySelector(".art svg")).not.toBeNull();
    expect(part("count")).toContain(String(personal?.count));
  });

  it("detail_empty_gives_way_to_the_selected_entry", async () => {
    const user = userEvent.setup();
    await openVault();
    await user.click(screen.getByRole("option", { name: /GitHub/ }));
    await screen.findByRole("heading", { name: "GitHub" });
    expect(screen.queryByTestId("detail-empty")).toBeNull();
  });

  it("detail_empty_passes_axe", async () => {
    await openVault();
    const region = screen.getByRole("region", { name: "Details" });
    expect(within(region).getByTestId("detail-empty")).toBeTruthy();
    await expectNoAxeViolations(region);
  });
});
