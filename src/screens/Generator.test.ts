import { fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import type { ComponentProps } from "svelte";
import { session } from "../lib/stores/session.svelte";
import { settings } from "../lib/stores/settings.svelte";
import { toasts } from "../lib/stores/toasts.svelte";
import { expectNoAxeViolations } from "../test/axe";
import { apiError, createFakeBackend, type FakeBackend } from "../test/fakeBackend";
import { sessionStatus } from "../test/fixtures";
import { calls, renderMainApp } from "../test/mainApp";
import Generator from "./Generator.svelte";

afterEach(() => {
  session.stop();
  settings.stop();
  toasts.clear();
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });
});

/** The generator alone on a fake backend, with the settings store loaded like a running window. */
async function mount(props: ComponentProps<typeof Generator> = {}, prepare?: (fake: FakeBackend) => void) {
  const fake = createFakeBackend({ session: "unlocked" });
  prepare?.(fake);
  fake.install();
  await session.start();
  await settings.load();
  const utils = render(Generator, { props });
  return { ...utils, fake };
}

const SITE_RULES = { minLength: null, maxLength: 15, maxConsecutive: 2, forcedOn: ["digits", "symbols"], excluded: [] };

const shown = () => screen.getByTestId("generated-value");

describe("controls", () => {
  it("length_slider_range_8_64_default_20", async () => {
    await mount();
    const slider = screen.getByRole("slider", { name: "Length" }) as HTMLInputElement;
    expect([slider.min, slider.max, slider.value]).toEqual(["8", "64", "20"]);
    const field = screen.getByRole("spinbutton", { name: "Length" }) as HTMLInputElement;
    expect(field.value).toBe("20");
    await fireEvent.input(slider, { target: { value: "33" } });
    expect((screen.getByRole("spinbutton", { name: "Length" }) as HTMLInputElement).value).toBe("33");
  });

  it("all_toggles_off_disables_generate", async () => {
    const user = userEvent.setup();
    await mount();
    for (const name of ["Lowercase", "Uppercase", "Digits", "Symbols"]) {
      expect(screen.getByRole("switch", { name }).getAttribute("aria-checked")).toBe("true");
    }
    const generate = screen.getByRole("button", { name: "Generate" }) as HTMLButtonElement;
    expect(generate.disabled).toBe(false);
    for (const name of ["Lowercase", "Uppercase", "Digits"]) await user.click(screen.getByRole("switch", { name }));
    expect(generate.disabled).toBe(false);
    await user.click(screen.getByRole("switch", { name: "Symbols" }));
    expect(generate.disabled).toBe(true);
  });

  it("changed_choices_are_remembered_through_settings", async () => {
    const user = userEvent.setup();
    const { fake } = await mount();
    await user.click(screen.getByRole("switch", { name: "Symbols" }));
    await waitFor(() => expect(fake.state.settings.generator.symbols).toBe(false));
    await fireEvent.change(screen.getByRole("slider", { name: "Length" }), { target: { value: "30" } });
    await waitFor(() => expect(fake.state.settings.generator.length).toBe(30));
    expect(fake.state.settings.generator).toEqual({ length: 30, lowercase: true, uppercase: true, digits: true, symbols: false });
  });

  it("remembered_defaults_prefill_the_controls", async () => {
    await mount({}, (fake) => {
      fake.state.settings.generator = { length: 12, lowercase: true, uppercase: false, digits: true, symbols: false };
    });
    expect((screen.getByRole("slider", { name: "Length" }) as HTMLInputElement).value).toBe("12");
    expect(screen.getByRole("switch", { name: "Uppercase" }).getAttribute("aria-checked")).toBe("false");
  });
});

describe("generating", () => {
  it("generate_displays_value_with_digit_and_symbol_spans", async () => {
    const user = userEvent.setup();
    const { fake } = await mount();
    expect(screen.queryByTestId("generated-value")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(shown().textContent).toHaveLength(20));
    const value = shown().textContent!;
    expect(shown().querySelectorAll('[data-class="digit"]').length).toBeGreaterThan(0);
    expect(shown().querySelectorAll('[data-class="symbol"]').length).toBeGreaterThan(0);
    const coloured = (cls: string) => [...shown().querySelectorAll(`[data-class="${cls}"]`)].map((n) => n.textContent).join("");
    expect(coloured("digit")).toBe(value.replace(/\D/g, ""));
    expect(coloured("symbol")).toBe(value.replace(/[A-Za-z0-9]/g, ""));
    // Display only: nothing is recorded in the history by generating.
    const req = calls(fake, "generatorGenerate")[0]!.args.req;
    expect(req).toEqual({ options: { length: 20, lowercase: true, uppercase: true, digits: true, symbols: true }, host: null, recordUrl: null });
    expect(fake.state.generatorHistory).toEqual([]);
  });

  it("rules_summary_for_host_forces_and_disables_toggles", async () => {
    await mount({ host: "limited.test" }, (fake) => {
      fake.state.generatorRules["limited.test"] = SITE_RULES;
    });
    const digits = await screen.findByRole("switch", { name: "Digits" });
    await waitFor(() => expect(digits.getAttribute("aria-disabled")).toBe("true"));
    for (const name of ["Digits", "Symbols"]) {
      const sw = screen.getByRole("switch", { name });
      expect(sw.getAttribute("aria-checked")).toBe("true");
      expect(sw.getAttribute("aria-disabled")).toBe("true");
    }
    expect(screen.getByRole("switch", { name: "Lowercase" }).getAttribute("aria-disabled")).not.toBe("true");
    expect((screen.getByRole("slider", { name: "Length" }) as HTMLInputElement).max).toBe("15");
    expect(screen.getByText(/This website requires:/).textContent).toContain("15");
  });

  it("excluded_classes_become_off_and_disabled", async () => {
    await mount({ host: "limited.test" }, (fake) => {
      fake.state.generatorRules["limited.test"] = { ...SITE_RULES, forcedOn: [], excluded: ["symbols"], maxLength: null, minLength: 12 };
    });
    const symbols = await screen.findByRole("switch", { name: "Symbols" });
    await waitFor(() => expect(symbols.getAttribute("aria-disabled")).toBe("true"));
    expect(symbols.getAttribute("aria-checked")).toBe("false");
    expect((screen.getByRole("slider", { name: "Length" }) as HTMLInputElement).min).toBe("12");
  });

  it("typing_a_website_looks_up_its_rules", async () => {
    const user = userEvent.setup();
    await mount({}, (fake) => {
      fake.state.generatorRules["limited.test"] = SITE_RULES;
    });
    expect(screen.queryByText(/This website requires:/)).toBeNull();
    await user.type(screen.getByRole("textbox", { name: "For website" }), "limited.test");
    await screen.findByText(/This website requires:/);
    await user.clear(screen.getByRole("textbox", { name: "For website" }));
    await waitFor(() => expect(screen.queryByText(/This website requires:/)).toBeNull());
  });

  it("effective_length_hint_shown_when_clamped", async () => {
    const user = userEvent.setup();
    await mount({ host: "limited.test" }, (fake) => {
      fake.state.generatorRules["limited.test"] = SITE_RULES;
    });
    await screen.findByText(/This website requires:/);
    expect(screen.queryByText("Limited to 15 characters by this website")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Generate" }));
    expect(await screen.findByText("Limited to 15 characters by this website")).toBeTruthy();
    expect(shown().textContent).toHaveLength(15);
  });

  it("generator_unavailable_error_shown_for_unsatisfiable_rules", async () => {
    const user = userEvent.setup();
    await mount({}, (fake) => {
      fake.handlers.generatorGenerate = () => {
        throw apiError("generatorUnsatisfiable");
      };
    });
    await user.click(screen.getByRole("button", { name: "Generate" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toBe("This site's password requirements could not be satisfied");
    expect(screen.queryByTestId("generated-value")).toBeNull();
  });

  it("a_no_class_error_from_the_backend_is_shown_too", async () => {
    const user = userEvent.setup();
    await mount({}, (fake) => {
      fake.handlers.generatorGenerate = () => {
        throw apiError("generatorNoClass");
      };
    });
    await user.click(screen.getByRole("button", { name: "Generate" }));
    expect((await screen.findByRole("alert")).textContent).toBe("Turn on at least one character type in the generator");
  });
});

describe("copy and use", () => {
  it("copy_calls_generator_copy_and_records_when_website_typed", async () => {
    const user = userEvent.setup();
    const { fake } = await mount();
    await user.type(screen.getByRole("textbox", { name: "For website" }), "  example.com ");
    await user.click(screen.getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(shown().textContent).toHaveLength(20));
    const value = shown().textContent!;
    await user.click(screen.getByRole("button", { name: "Copy" }));
    await waitFor(() => expect(calls(fake, "generatorCopy")).toHaveLength(1));
    expect(calls(fake, "generatorCopy")[0]!.args).toEqual({ value, recordUrl: "example.com" });
    expect(fake.state.generatorHistory.map((h) => h.url)).toEqual(["example.com"]);
    // The toast never carries the value.
    expect(toasts.items.map((x) => x.text).join("|")).not.toContain(value);
  });

  it("copy_without_website_passes_no_record_url", async () => {
    const user = userEvent.setup();
    const { fake } = await mount();
    await user.click(screen.getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(shown().textContent).toHaveLength(20));
    await user.click(screen.getByRole("button", { name: "Copy" }));
    await waitFor(() => expect(calls(fake, "generatorCopy")).toHaveLength(1));
    expect(calls(fake, "generatorCopy")[0]!.args.recordUrl).toBeNull();
    expect(fake.state.generatorHistory).toEqual([]);
  });

  it("the_page_url_of_the_form_is_the_record_url_while_the_website_is_unchanged", async () => {
    const user = userEvent.setup();
    const { fake } = await mount({ host: "example.com", recordUrl: "https://example.com/login" });
    expect((screen.getByRole("textbox", { name: "For website" }) as HTMLInputElement).value).toBe("example.com");
    await user.click(screen.getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(shown().textContent).toHaveLength(20));
    await user.click(screen.getByRole("button", { name: "Copy" }));
    await waitFor(() => expect(calls(fake, "generatorCopy")).toHaveLength(1));
    expect(calls(fake, "generatorCopy")[0]!.args.recordUrl).toBe("https://example.com/login");
  });

  it("copy_and_use_are_disabled_until_a_value_exists", async () => {
    let used: string | null = null;
    await mount({ onuse: (v) => (used = v) });
    expect((screen.getByRole("button", { name: "Copy" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "Use password" }) as HTMLButtonElement).disabled).toBe(true);
    expect(used).toBeNull();
  });

  it("use_button_returns_value_to_form_and_closes", async () => {
    // Through the real editor: Generate opens the generator, Use fills the password and closes it.
    const user = userEvent.setup();
    await renderMainApp({ session: "unlocked" });
    await user.click(await screen.findByRole("button", { name: "New Entry" }));
    await user.click(await screen.findByRole("menuitem", { name: "New Login" }));
    await user.click(await screen.findByRole("button", { name: "Generate" }));
    // The form's own Generate button comes first in the DOM, the generator's second.
    await user.click(screen.getAllByRole("button", { name: "Generate" })[1]!);
    await waitFor(() => expect(shown().textContent).toHaveLength(20));
    const value = shown().textContent!;
    await user.click(screen.getByRole("button", { name: "Use password" }));
    expect(screen.queryByTestId("generated-value")).toBeNull();
    expect((screen.getByLabelText("Password") as HTMLInputElement).value).toBe(value);
  });

  it("modal_mode_has_no_use_button", async () => {
    await mount();
    expect(screen.queryByRole("button", { name: "Use password" })).toBeNull();
  });
});

describe("secrets in the webview", () => {
  it("generated_value_cleared_on_lock", async () => {
    const user = userEvent.setup();
    const { fake } = await mount();
    await user.click(screen.getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(shown().textContent).toHaveLength(20));
    await fake.emit("sessionChanged", sessionStatus("locked"));
    await waitFor(() => expect(screen.queryByTestId("generated-value")).toBeNull());
  });

  it("generated_value_cleared_when_the_window_hides", async () => {
    const user = userEvent.setup();
    await mount();
    await user.click(screen.getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(shown().textContent).toHaveLength(20));
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    await fireEvent(document, new Event("visibilitychange"));
    await waitFor(() => expect(screen.queryByTestId("generated-value")).toBeNull());
  });

  it("generated_value_is_gone_after_the_component_is_destroyed", async () => {
    const user = userEvent.setup();
    const { unmount } = await mount();
    await user.click(screen.getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(shown().textContent).toHaveLength(20));
    const value = shown().textContent!;
    unmount();
    expect(document.body.textContent).not.toContain(value);
  });
});

describe("history", () => {
  const seed = (fake: FakeBackend) => {
    fake.state.generatorHistory.push(
      { id: "h1", url: "https://old.example/login", title: "old.example", created: 1_700_000_000_000, password: "OLD-secret-h1" },
      { id: "h2", url: "example.org", title: "example.org", created: 1_700_100_000_000, password: "NEW-secret-h2" },
    );
  };

  async function openHistory(prepare = seed) {
    const user = userEvent.setup();
    const utils = await mount({}, prepare);
    await user.click(screen.getByRole("radio", { name: "Recently generated" }));
    const list = prepare === seed ? await screen.findByRole("list", { name: "Generated passwords" }) : document.body;
    return { ...utils, user, list };
  }

  it("history_list_masked_with_reveal_copy_and_delete", async () => {
    const { fake, user, list } = await openHistory();
    const rows = within(list).getAllByRole("listitem");
    expect(rows).toHaveLength(2);
    expect(rows[0]!.textContent).toContain("example.org");
    expect(rows[1]!.textContent).toContain("old.example");
    expect(document.body.textContent).not.toContain("secret-h");
    await user.click(within(rows[0]!).getByRole("button", { name: "Reveal" }));
    expect(await within(rows[0]!).findByText("NEW-secret-h2")).toBeTruthy();
    // One secret at a time: revealing the second hides the first.
    await user.click(within(rows[1]!).getByRole("button", { name: "Reveal" }));
    expect(await within(rows[1]!).findByText("OLD-secret-h1")).toBeTruthy();
    expect(document.body.textContent).not.toContain("NEW-secret-h2");
    await user.click(within(rows[1]!).getByRole("button", { name: "Hide" }));
    expect(document.body.textContent).not.toContain("OLD-secret-h1");
    await user.click(within(rows[0]!).getByRole("button", { name: "Copy" }));
    await waitFor(() => expect(calls(fake, "generatorHistoryCopy").map((c) => c.args.id)).toEqual(["h2"]));
    expect(document.body.textContent).not.toContain("secret-h");
  });

  it("history_delete_calls_backend_and_removes_row", async () => {
    const { fake, user, list } = await openHistory();
    await user.click(within(within(list).getAllByRole("listitem")[0]!).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(within(list).getAllByRole("listitem")).toHaveLength(1));
    expect(calls(fake, "generatorHistoryDelete").map((c) => c.args.id)).toEqual(["h2"]);
    expect(fake.state.generatorHistory.map((h) => h.id)).toEqual(["h1"]);
  });

  it("empty_history_says_so", async () => {
    await openHistory(() => undefined);
    expect(screen.getByText("No generated passwords yet")).toBeTruthy();
  });

  it("a_revealed_history_value_is_cleared_when_the_window_hides_and_when_leaving_the_tab", async () => {
    const { user, list } = await openHistory();
    await user.click(within(within(list).getAllByRole("listitem")[0]!).getByRole("button", { name: "Reveal" }));
    await screen.findByText("NEW-secret-h2");
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    await fireEvent(document, new Event("visibilitychange"));
    await waitFor(() => expect(document.body.textContent).not.toContain("NEW-secret-h2"));
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });
    await user.click(within(within(list).getAllByRole("listitem")[0]!).getByRole("button", { name: "Reveal" }));
    await screen.findByText("NEW-secret-h2");
    await user.click(screen.getByRole("radio", { name: "Generate" }));
    expect(document.body.textContent).not.toContain("NEW-secret-h2");
  });

  it("copy_with_a_website_shows_up_in_the_history_tab", async () => {
    const user = userEvent.setup();
    await mount();
    await user.type(screen.getByRole("textbox", { name: "For website" }), "fresh.example");
    await user.click(screen.getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(shown().textContent).toHaveLength(20));
    await user.click(screen.getByRole("button", { name: "Copy" }));
    await user.click(screen.getByRole("radio", { name: "Recently generated" }));
    const list = await screen.findByRole("list", { name: "Generated passwords" });
    expect(within(list).getAllByRole("listitem")).toHaveLength(1);
    expect(list.textContent).toContain("fresh.example");
  });
});

describe("toolbar entry", () => {
  it("toolbar_menu_opens_the_generator_in_a_dialog_without_use", async () => {
    const user = userEvent.setup();
    await renderMainApp({ session: "unlocked" });
    await user.click(await screen.findByRole("button", { name: "New Entry" }));
    await user.click(await screen.findByRole("menuitem", { name: "Password Generator…" }));
    const dialog = await screen.findByRole("dialog", { name: "Password Generator…" });
    expect(within(dialog).queryByRole("button", { name: "Use password" })).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(within(dialog).getByTestId("generated-value").textContent).toHaveLength(20));
    await user.click(within(dialog).getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByTestId("generated-value")).toBeNull();
  });
});

describe("accessibility", () => {
  it("generator_passes_axe", async () => {
    const user = userEvent.setup();
    const { container } = await mount({ host: "limited.test", onuse: () => undefined }, (fake) => {
      fake.state.generatorRules["limited.test"] = SITE_RULES;
      fake.state.generatorHistory.push({ id: "h1", url: "x.test", title: "x.test", created: 1_700_000_000_000, password: "p" });
    });
    await screen.findByText(/This website requires:/);
    await user.click(screen.getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(shown().textContent).toHaveLength(15));
    await expectNoAxeViolations(container);
    await user.click(screen.getByRole("radio", { name: "Recently generated" }));
    await screen.findByRole("list", { name: "Generated passwords" });
    await expectNoAxeViolations(container);
  });
});
