import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { session } from "../lib/stores/session.svelte";
import { createFakeBackend } from "../test/fakeBackend";
import { FIXTURE_LOGIN_PASSWORD } from "../test/fixtures";
import { expectNoAxeViolations } from "../test/axe";
import { renderWithBackend } from "../test/render";
import LargeType from "./LargeType.svelte";

afterEach(() => {
  session.stop();
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });
});

async function open(onclose = vi.fn(), field: { kind: "password" } | { kind: "totp" } = { kind: "password" }) {
  const fake = createFakeBackend({ session: "unlocked" });
  const view = renderWithBackend(LargeType, { entryId: "e-github", field, onclose }, fake);
  await session.start();
  return { ...view, fake, onclose };
}

describe("LargeType", () => {
  it("shows_the_value_in_a_named_modal_dialog_after_one_reveal", async () => {
    const { fake } = await open();
    const dialog = await screen.findByRole("dialog", { name: "Large Type" });
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    await waitFor(() => expect(dialog.textContent).toContain(FIXTURE_LOGIN_PASSWORD));
    expect(fake.calls.filter((c) => c.command === "entryReveal")).toEqual([
      { command: "entryReveal", args: { id: "e-github", field: { kind: "password" } } },
    ]);
  });

  it("colours_digits_and_symbols_by_class_with_decorative_spans", async () => {
    const fake = createFakeBackend({ session: "unlocked" });
    fake.state.entries[0]!.model.password = "ab12-c";
    fake.install();
    render(LargeType, { props: { entryId: "e-github", field: { kind: "password" }, onclose: vi.fn() } });
    const dialog = await screen.findByRole("dialog", { name: "Large Type" });
    await waitFor(() => expect(dialog.textContent).toContain("ab12-c"));
    const classes = Array.from(dialog.querySelectorAll("[data-class]")).map((s) => [s.getAttribute("data-class"), s.textContent]);
    expect(classes).toEqual([["letter", "ab"], ["digit", "12"], ["symbol", "-"], ["letter", "c"]]);
    for (const span of dialog.querySelectorAll("[data-class]")) expect(span.hasAttribute("aria-label")).toBe(false);
  });

  it("escape_closes", async () => {
    const { onclose } = await open();
    await screen.findByRole("dialog", { name: "Large Type" });
    await userEvent.setup().keyboard("{Escape}");
    expect(onclose).toHaveBeenCalled();
  });

  it("a_click_on_the_backdrop_closes_and_a_click_inside_does_not", async () => {
    const { onclose } = await open();
    const dialog = await screen.findByRole("dialog", { name: "Large Type" });
    await userEvent.setup().click(dialog);
    expect(onclose).not.toHaveBeenCalled();
    const backdrop = dialog.parentElement!;
    await fireEvent.pointerDown(backdrop);
    await fireEvent.click(backdrop);
    expect(onclose).toHaveBeenCalled();
  });

  it("a_hidden_window_closes", async () => {
    const { onclose } = await open();
    await screen.findByRole("dialog", { name: "Large Type" });
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    document.dispatchEvent(new Event("visibilitychange"));
    expect(onclose).toHaveBeenCalled();
  });

  it("a_lock_epoch_change_closes_and_drops_the_value", async () => {
    const { onclose } = await open();
    const dialog = await screen.findByRole("dialog", { name: "Large Type" });
    await waitFor(() => expect(dialog.textContent).toContain(FIXTURE_LOGIN_PASSWORD));
    session.lockEpoch += 1;
    await waitFor(() => expect(onclose).toHaveBeenCalled());
    expect(document.body.textContent).not.toContain(FIXTURE_LOGIN_PASSWORD);
  });

  it("a_failed_reveal_closes_the_dialog", async () => {
    const fake = createFakeBackend({ session: "unlocked" });
    fake.install();
    const onclose = vi.fn();
    render(LargeType, { props: { entryId: "missing", field: { kind: "password" }, onclose } });
    await waitFor(() => expect(onclose).toHaveBeenCalled());
  });

  it("passes_axe", async () => {
    await open();
    const dialog = await screen.findByRole("dialog", { name: "Large Type" });
    await waitFor(() => expect(dialog.textContent).toContain(FIXTURE_LOGIN_PASSWORD));
    await expectNoAxeViolations(document.body);
  });
});
