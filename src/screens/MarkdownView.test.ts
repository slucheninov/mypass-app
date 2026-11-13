import { fireEvent, render } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }));
import { openUrl } from "@tauri-apps/plugin-opener";
import MarkdownView from "./MarkdownView.svelte";

beforeEach(() => vi.mocked(openUrl).mockClear());

const SOURCE = "[web](https://a.test/x) [mail](mailto:a@b.test) [bad](javascript:alert(1))";

function link(container: HTMLElement, text: string): HTMLElement {
  const found = Array.from(container.querySelectorAll("a")).find((a) => a.textContent === text);
  if (!found) throw new Error(`no link ${text}`);
  return found;
}

describe("links in rendered markdown", () => {
  it("primary_click_opens_http_links_through_the_opener_and_never_navigates", async () => {
    const { container } = render(MarkdownView, { source: SOURCE });
    const notPrevented = await fireEvent.click(link(container, "web"));
    expect(notPrevented).toBe(false);
    expect(openUrl).toHaveBeenCalledWith("https://a.test/x");
  });

  it("middle_click_is_intercepted_and_opens_through_the_opener", async () => {
    const { container } = render(MarkdownView, { source: SOURCE });
    const notPrevented = await fireEvent(link(container, "web"), new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 }));
    expect(notPrevented).toBe(false);
    expect(openUrl).toHaveBeenCalledTimes(1);
  });

  it("right_button_auxclick_and_context_menu_are_intercepted_and_open_nothing", async () => {
    const { container } = render(MarkdownView, { source: SOURCE });
    const a = link(container, "web");
    expect(await fireEvent(a, new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 2 }))).toBe(false);
    expect(await fireEvent.contextMenu(a)).toBe(false);
    expect(openUrl).not.toHaveBeenCalled();
  });

  it("enter_on_a_focused_link_opens_it_and_cancels_the_default_activation", async () => {
    const { container } = render(MarkdownView, { source: SOURCE });
    const notPrevented = await fireEvent.keyDown(link(container, "web"), { key: "Enter" });
    expect(notPrevented).toBe(false);
    expect(openUrl).toHaveBeenCalledWith("https://a.test/x");
  });

  it("dragging_a_link_is_cancelled", async () => {
    const { container } = render(MarkdownView, { source: SOURCE });
    expect(await fireEvent.dragStart(link(container, "web"))).toBe(false);
  });

  it("mailto_and_unsafe_schemes_render_as_text_with_no_href_and_do_nothing", async () => {
    const { container } = render(MarkdownView, { source: SOURCE });
    for (const text of ["mail", "bad"]) {
      const a = link(container, text);
      expect(a.hasAttribute("href"), text).toBe(false);
      await fireEvent.click(a);
      await fireEvent.keyDown(a, { key: "Enter" });
    }
    expect(openUrl).not.toHaveBeenCalled();
  });
});
