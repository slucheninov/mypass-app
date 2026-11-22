import { screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import type { AddressDto, PersonDto } from "../lib/bindings";
import { categoryIcon, categoryLabel } from "../lib/categories";
import { setLanguage, t } from "../lib/i18n/index.svelte";
import { navigation } from "../lib/stores/navigation.svelte";
import { expectNoAxeViolations } from "../test/axe";
import { backendWith, openEdit, openNew, user } from "../test/editorHelpers";
import { fixtureSidebar, type EntrySpec } from "../test/fixtures";
import { calls, renderMainApp } from "../test/mainApp";

afterEach(() => setLanguage("en"));

const blankAddress = (patch: Partial<AddressDto>): AddressDto => ({
  label: "Home", fullName: "", givenName: "", familyName: "", organization: "", addressLine1: "", addressLine2: "",
  city: "", region: "", postalCode: "", countryCode: "", countryName: "", phone: "", email: "", ...patch,
});

const person = (patch: Partial<PersonDto> = {}): PersonDto => ({
  label: "", givenName: "", familyName: "", middleName: "", email: "", birthDate: "", addressIds: [], ...patch,
});

const ADDRESSES: EntrySpec[] = [
  { id: "a-home", title: "Home", subtitle: "Kyiv", entryType: "address", modified: 1, address: blankAddress({ city: "Kyiv" }) },
  { id: "a-work", title: "Work", subtitle: "Lviv", entryType: "address", modified: 2, address: blankAddress({ label: "Work", city: "Lviv" }) },
  { id: "a-old", title: "Old", entryType: "address", modified: 3, inRecycleBin: true, groupId: "g-bin", address: blankAddress({}) },
];

const ANN: EntrySpec = {
  id: "p-ann",
  title: "Ann",
  subtitle: "ann@example.org",
  entryType: "person",
  modified: 9,
  person: person({
    label: "Ann",
    givenName: "Ann",
    middleName: "Maria",
    familyName: "Doe",
    email: "ann@example.org",
    birthDate: "1990-02-03",
    addressIds: ["a-missing", "a-old", "a-work", "a-home"],
  }),
};

const savedPerson = (fake: { calls: { command: string; args: Record<string, any> }[] }) =>
  fake.calls.filter((c) => c.command === "entrySave")[0]?.args.model.person as PersonDto;
const box = (label: string) => screen.getByRole("textbox", { name: label }) as HTMLInputElement;
const links = () => within(screen.getByRole("region", { name: "Linked addresses" }));

describe("people category", () => {
  it("people_sits_after_addresses_with_its_own_icon_label_and_count", async () => {
    expect(categoryLabel("people")).toBe("People");
    expect(categoryIcon("people")).toBe("person");
    expect(categoryIcon("people")).not.toBe(categoryIcon("addresses"));
    const fake = backendWith([...ADDRESSES, ANN, { ...ANN, id: "p-bob", title: "Bob" }]);
    const counts = fixtureSidebar(fake.state.entries).categories.map((c) => c.category);
    expect(counts.indexOf("people")).toBe(counts.indexOf("addresses") + 1);
    await renderMainApp(fake);
    await screen.findByRole("listbox");
    const item = within(screen.getByRole("group", { name: "Categories" })).getByRole("treeitem", { name: /People/ });
    expect(item.textContent).toContain("2");
    navigation.setScope({ kind: "category", category: "people" });
    await waitFor(() =>
      expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual([
        expect.stringContaining("Ann"),
        expect.stringContaining("Bob"),
      ]),
    );
  });

  it("the_new_menu_offers_a_person", async () => {
    const fake = backendWith();
    await renderMainApp(fake);
    await screen.findByRole("listbox");
    await user().click(screen.getByRole("button", { name: "New Entry" }));
    await user().click(await screen.findByRole("menuitem", { name: "New Person" }));
    await screen.findByRole("heading", { name: "New Person" });
    expect(calls(fake, "entryNewModel").at(-1)?.args.entryType).toBe("person");
  });
});

describe("person editor", () => {
  it("fields_and_ordered_links_map_to_the_model", async () => {
    const fake = backendWith(ADDRESSES);
    await openNew(fake, "person");
    await user().type(box("Given Name"), "Ann");
    await user().type(box("Middle name"), "Maria");
    await user().type(box("Family Name"), "Doe");
    await user().type(box("Email"), "ann@example.org");
    const date = screen.getByLabelText("Birth date") as HTMLInputElement;
    expect(date.type).toBe("date");
    await user().type(date, "1990-02-03");
    // The binned address is not offered; linking keeps the order of the clicks.
    expect(await links().findByRole("button", { name: "Link Work" })).toBeTruthy();
    expect(links().queryByRole("button", { name: "Link Old" })).toBeNull();
    await user().click(links().getByRole("button", { name: "Link Work" }));
    await user().click(links().getByRole("button", { name: "Link Home" }));
    expect(links().getByText("Default")).toBeTruthy();
    await user().click(links().getByRole("button", { name: "Set Home as default" }));
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "entrySave")).toHaveLength(1));
    expect(savedPerson(fake)).toEqual({
      label: "",
      givenName: "Ann",
      middleName: "Maria",
      familyName: "Doe",
      email: "ann@example.org",
      birthDate: "1990-02-03",
      addressIds: ["a-home", "a-work"],
    });
    // An untitled person takes its full name.
    expect(fake.state.entries.at(-1)?.summary.title).toBe("Ann Maria Doe");
  });

  it("editing_hides_gone_links_and_drops_them_on_the_next_change", async () => {
    const fake = backendWith([...ADDRESSES, ANN]);
    await openEdit(fake, "p-ann");
    expect(box("Middle name").value).toBe("Maria");
    const rows = await waitFor(() => {
      const items = links().getAllByRole("listitem");
      expect(items.length).toBeGreaterThan(1);
      return items;
    });
    expect(rows[0]?.textContent).toContain("Work");
    expect(rows[1]?.textContent).toContain("Home");
    await user().click(links().getByRole("button", { name: "Unlink Work" }));
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "entrySave")).toHaveLength(1));
    expect(savedPerson(fake).addressIds).toEqual(["a-home"]);
  });

  it("a_malformed_birth_date_blocks_save_with_an_inline_error", async () => {
    const fake = backendWith([...ADDRESSES, { ...ANN, person: person({ givenName: "Ann", birthDate: "1990-2-3" }) }]);
    await openEdit(fake, "p-ann");
    await user().click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Enter a valid birth date")).toBeTruthy();
    expect(calls(fake, "entrySave")).toHaveLength(0);
  });

  it("the_form_passes_axe", async () => {
    const fake = backendWith(ADDRESSES);
    await openNew(fake, "person");
    await links().findByRole("button", { name: "Link Home" });
    await expectNoAxeViolations(document.body);
  });
});

describe("person detail", () => {
  async function openAnn(language: "en" | "uk" = "en") {
    const fake = backendWith([...ADDRESSES, ANN]);
    fake.state.settings.language = language;
    await renderMainApp(fake);
    await screen.findByRole("listbox");
    navigation.select("p-ann");
    await screen.findByRole("heading", { name: "Ann" });
    return fake;
  }
  const pane = () => screen.getByRole("region", { name: t("Details") });
  const row = (label: string) => within(pane()).getByRole("group", { name: label });

  it("lists_the_fields_and_only_live_linked_addresses_in_order", async () => {
    const fake = await openAnn();
    for (const [label, value] of <[string, string][]>[
      ["Given Name", "Ann"],
      ["Middle name", "Maria"],
      ["Family Name", "Doe"],
      ["Email", "ann@example.org"],
      ["Birth date", "1990-02-03"],
    ]) {
      expect(row(label).textContent, label).toContain(value);
    }
    const linked = within(row("Linked addresses")).getAllByRole("button").map((b) => b.textContent);
    expect(linked).toEqual(["Work", "Home"]);
    expect(row("Linked addresses").textContent).toContain("Default");
    expect(pane().textContent).not.toContain("Old");
    await expectNoAxeViolations(pane());
    await userEvent.setup().click(within(row("Email")).getByRole("button", { name: "ann@example.org" }));
    expect(calls(fake, "entryCopy").map((c) => c.args)).toEqual([
      { id: "p-ann", field: { kind: "person", name: "email" } },
    ]);
  });

  it("a_linked_address_opens_that_entry", async () => {
    await openAnn();
    await userEvent.setup().click(within(row("Linked addresses")).getByRole("button", { name: "Home" }));
    expect(navigation.scope).toEqual({ kind: "category", category: "addresses" });
    await screen.findByRole("heading", { name: "Home" });
    expect(navigation.selectedId).toBe("a-home");
  });

  it("shows_middle_name_in_ukrainian_as_po_batkovi", async () => {
    await openAnn("uk");
    const name = "\u041f\u043e \u0431\u0430\u0442\u044c\u043a\u043e\u0432\u0456";
    await waitFor(() => expect(within(pane()).getByRole("group", { name })).toBeTruthy());
    expect(row(name).textContent).toContain("Maria");
  });
});
