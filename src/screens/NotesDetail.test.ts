import { emit } from "@tauri-apps/api/event";
import { mockWindows } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createRevealGroup, type RevealGroup } from "../lib/secret.svelte";
import { session } from "../lib/stores/session.svelte";
import { createFakeBackend, type FakeBackend } from "../test/fakeBackend";
import { renderWithBackend } from "../test/render";
import NotesDetail from "./NotesDetail.svelte";

const NOTES = "**bold** <script>alert(1)</script> <img src=x onerror=alert(1)> plus [a](javascript:alert(1)) [b](https://b.test)";

const groups: RevealGroup[] = [];

afterEach(() => {
  for (const g of groups.splice(0)) g.destroy();
  session.stop();
  vi.useRealTimers();
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });
});

async function open(fake: FakeBackend = createFakeBackend({ session: "unlocked" })) {
  const entry = fake.state.entries.find((e) => e.summary.id === "e-notes")!;
  entry.model.notes = NOTES;
  const group = createRevealGroup();
  groups.push(group);
  const view = renderWithBackend(NotesDetail, { entryId: "e-notes", group }, fake);
  await session.start();
  return { ...view, fake, group };
}

const reveal = async () => userEvent.setup().click(screen.getByRole("button", { name: "Reveal notes" }));

describe("NotesDetail", () => {
  it("notes_hidden_until_reveal_then_markdown_sanitised", async () => {
    const { container, fake } = await open();
    expect(screen.getByText("Protected notes")).toBeTruthy();
    expect(container.textContent).not.toContain("bold");
    expect(fake.calls.filter((c) => c.command === "entryReveal")).toHaveLength(0);
    await reveal();
    await waitFor(() => expect(container.querySelector("strong")?.textContent).toBe("bold"));
    expect(fake.calls.filter((c) => c.command === "entryReveal")).toEqual([
      { command: "entryReveal", args: { id: "e-notes", field: { kind: "notesBody" } } },
    ]);
    expect(container.innerHTML).not.toContain("<script");
    expect(container.innerHTML).not.toContain("onerror");
    expect(container.querySelector("img")).toBeNull();
    expect(screen.getByRole("button", { name: "Hide notes" })).toBeTruthy();
  });

  it("hide_notes_removes_the_text", async () => {
    const { container } = await open();
    await reveal();
    await waitFor(() => expect(container.querySelector("strong")).not.toBeNull());
    await userEvent.setup().click(screen.getByRole("button", { name: "Hide notes" }));
    expect(container.textContent).not.toContain("bold");
  });

  it("links_open_through_the_opener_only_for_http_and_https", async () => {
    const opened: unknown[] = [];
    const fake = createFakeBackend({ session: "unlocked", plugins: { "plugin:opener|open_url": (p) => (opened.push(p), null) } });
    const { container } = await open(fake);
    await reveal();
    await waitFor(() => expect(container.querySelector("a")).not.toBeNull());
    const user = userEvent.setup();
    await user.click(screen.getByText("b"));
    await waitFor(() => expect(opened).toHaveLength(1));
    expect(JSON.stringify(opened[0])).toContain("https://b.test");
    const bad = screen.getByText("a");
    expect(bad.getAttribute("href")).toBeNull();
    await user.click(bad);
    expect(opened).toHaveLength(1);
  });

  it("notes_rehide_on_window_minimize_or_hide", async () => {
    mockWindows("main");
    const { container, fake, group } = await open(createFakeBackend({ session: "unlocked" }));
    await reveal();
    await waitFor(() => expect(container.querySelector("strong")).not.toBeNull());
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    document.dispatchEvent(new Event("visibilitychange"));
    await waitFor(() => expect(container.textContent).not.toContain("bold"));
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });

    await reveal();
    await waitFor(() => expect(container.querySelector("strong")).not.toBeNull());
    fake.state.minimized = true;
    await emit("tauri://resize", { width: 1, height: 1 });
    await waitFor(() => expect(container.textContent).not.toContain("bold"));
    // The group holds no value: a member reveals again from scratch.
    const probe = group.create();
    expect(probe.value).toBeNull();
  });

  it("notes_hide_after_sixty_seconds_of_window_blur_and_a_focus_cancels_it", async () => {
    const { container } = await open();
    await reveal();
    await waitFor(() => expect(container.querySelector("strong")).not.toBeNull());
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    await fireEvent.blur(window);
    await vi.advanceTimersByTimeAsync(59_000);
    expect(container.querySelector("strong")).not.toBeNull();
    await fireEvent.focus(window);
    await vi.advanceTimersByTimeAsync(5_000);
    expect(container.querySelector("strong")).not.toBeNull();
    await fireEvent.blur(window);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(container.textContent).not.toContain("bold");
  });

  it("a_lock_hides_the_notes", async () => {
    const { container } = await open();
    await reveal();
    await waitFor(() => expect(container.querySelector("strong")).not.toBeNull());
    session.lockEpoch += 1;
    await waitFor(() => expect(container.textContent).not.toContain("bold"));
  });
});
