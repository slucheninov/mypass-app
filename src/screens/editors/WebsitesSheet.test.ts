import { render, screen, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { EntryEditModel_Serialize } from "../../lib/bindings";
import { expectNoAxeViolations } from "../../test/axe";
import WebsitesSheet from "./WebsitesSheet.svelte";

function model(urls: string[], mode = "registrable-domain"): EntryEditModel_Serialize {
  return {
    id: "e1",
    entryType: "login",
    groupId: "g",
    title: "T",
    favorite: false,
    username: "",
    password: "",
    urls,
    uriMatchMode: mode,
    totp: "",
    notes: "",
    address: null,
    card: null,
    customFields: [],
    attachments: [],
    attachmentOps: [],
  };
}

const inputs = () => screen.getAllByRole("textbox", { name: "URL" }) as HTMLInputElement[];

describe("WebsitesSheet", () => {
  it("websites_sheet_adds_removes_and_reorders_urls", async () => {
    const m = model(["https://a.test", "https://b.test"]);
    const onsave = vi.fn();
    render(WebsitesSheet, { model: m, onsave, oncancel: vi.fn() });
    const u = userEvent.setup();
    expect(inputs().map((i) => i.value)).toEqual(["https://a.test", "https://b.test"]);
    // Reorder: the second row up, then the (new) second row down is a no-op at the end.
    await u.click(screen.getAllByRole("button", { name: "Move up" })[1]!);
    expect(inputs().map((i) => i.value)).toEqual(["https://b.test", "https://a.test"]);
    await u.click(screen.getAllByRole("button", { name: "Move down" })[0]!);
    expect(inputs().map((i) => i.value)).toEqual(["https://a.test", "https://b.test"]);
    // The first row cannot go up and the last cannot go down.
    expect((screen.getAllByRole("button", { name: "Move up" })[0] as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getAllByRole("button", { name: "Move down" })[1] as HTMLButtonElement).disabled).toBe(true);
    // Add
    await u.click(screen.getByRole("button", { name: "Add Website" }));
    await u.type(inputs()[2]!, "  https://c.test ");
    // Remove the first
    await u.click(screen.getAllByRole("button", { name: "Remove" })[0]!);
    expect(inputs().map((i) => i.value)).toEqual(["https://b.test", "  https://c.test "]);
    await u.click(screen.getByRole("button", { name: "Save" }));
    expect(m.urls).toEqual(["https://b.test", "https://c.test"]);
    expect(onsave).toHaveBeenCalledOnce();
  });

  it("uri_match_mode_select_has_five_options_in_spec_order", async () => {
    const m = model(["https://a.test"]);
    render(WebsitesSheet, { model: m, onsave: vi.fn(), oncancel: vi.fn() });
    const select = screen.getByRole("combobox", { name: "Match mode" });
    const options = within(select).getAllByRole("option") as HTMLOptionElement[];
    expect(options.map((o) => o.value)).toEqual(["registrable-domain", "host", "starts-with", "exact", "never"]);
    expect(options.map((o) => o.textContent)).toEqual(["Same site", "Same host", "Starts with", "Exact URL", "Never offer"]);
  });

  it("every_mode_has_a_one_line_explanation_and_the_choice_is_saved", async () => {
    const m = model(["https://a.test"]);
    render(WebsitesSheet, { model: m, onsave: vi.fn(), oncancel: vi.fn() });
    const u = userEvent.setup();
    const seen = new Set<string>();
    for (const mode of ["registrable-domain", "host", "starts-with", "exact", "never"]) {
      await u.selectOptions(screen.getByRole("combobox", { name: "Match mode" }), mode);
      const note = document.querySelector("[data-testid='mode-explanation']")?.textContent ?? "";
      expect(note).not.toBe("");
      expect(note).not.toContain("\n");
      seen.add(note);
    }
    expect(seen.size).toBe(5);
    await u.click(screen.getByRole("button", { name: "Save" }));
    expect(m.uriMatchMode).toBe("never");
  });

  it("cancel_leaves_the_model_alone", async () => {
    const m = model(["https://a.test"]);
    const oncancel = vi.fn();
    render(WebsitesSheet, { model: m, onsave: vi.fn(), oncancel });
    await userEvent.setup().type(inputs()[0]!, "x");
    await userEvent.setup().click(screen.getByRole("button", { name: "Cancel" }));
    expect(m.urls).toEqual(["https://a.test"]);
    expect(oncancel).toHaveBeenCalledOnce();
  });

  it("websites_sheet_passes_axe", async () => {
    render(WebsitesSheet, { model: model(["https://a.test"]), onsave: vi.fn(), oncancel: vi.fn() });
    await expectNoAxeViolations(document.body);
  });
});
