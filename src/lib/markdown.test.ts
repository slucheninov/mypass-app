import { describe, expect, it } from "vitest";
import { renderMarkdown } from "./markdown";

const parse = (html: string) => {
  const host = document.createElement("div");
  host.append(...new DOMParser().parseFromString(html, "text/html").body.childNodes);
  return host;
};

describe("renderMarkdown", () => {
  it("renders_basic_markup", () => {
    const host = parse(renderMarkdown("# Title\n\n**bold** and *em* and `code`\n\n- one\n- two\n\n> quote"));
    expect(host.querySelector("h1")?.textContent).toBe("Title");
    expect(host.querySelector("strong")?.textContent).toBe("bold");
    expect(host.querySelector("em")?.textContent).toBe("em");
    expect(host.querySelector("code")?.textContent).toBe("code");
    expect(host.querySelectorAll("li")).toHaveLength(2);
    expect(host.querySelector("blockquote")).not.toBeNull();
  });

  it("scripts_event_handlers_and_images_are_removed", () => {
    const html = renderMarkdown("**bold** <script>alert(1)</script> <img src=x onerror=alert(1)>");
    expect(html).toContain("<strong>bold</strong>");
    expect(html).not.toContain("script");
    expect(html).not.toContain("onerror");
    expect(html).not.toContain("<img");
  });

  it("style_form_iframe_and_inline_attributes_never_survive", () => {
    const html = renderMarkdown(
      '<style>*{display:none}</style><form action="x"><input name="p"></form><iframe src="x"></iframe><p style="color:red" onclick="x()">hi</p>',
    );
    for (const bad of ["<style", "<form", "<input", "<iframe", "style=", "onclick"]) expect(html, bad).not.toContain(bad);
    expect(html).toContain("hi");
  });

  it("markdown_links_get_rel_noopener_and_javascript_urls_removed", () => {
    const host = parse(
      renderMarkdown(
        "[ok](https://a.test/x) [plain](http://b.test) [mail](mailto:a@b.test) [bad](javascript:alert(1)) [data](data:text/html,x) [file](file:///etc/passwd)\n\n<a href=\"JaVaScRiPt:alert(1)\" target=\"_blank\">raw</a>",
      ),
    );
    const links = Array.from(host.querySelectorAll("a"));
    const byText = (text: string) => links.find((a) => a.textContent === text);
    for (const text of ["ok", "plain"]) {
      expect(byText(text)?.getAttribute("href"), text).toBeTruthy();
      expect(byText(text)?.getAttribute("rel"), text).toBe("noopener noreferrer");
    }
    for (const text of ["mail", "bad", "data", "file", "raw"]) {
      expect(byText(text)?.hasAttribute("href") ?? false, text).toBe(false);
    }
    for (const a of links) expect(a.hasAttribute("target")).toBe(false);
  });

  it("only_href_and_rel_survive_on_any_element", () => {
    const host = parse(
      renderMarkdown(
        [
          '<p data-tauri-drag-region aria-label="x" aria-hidden="true" role="button" id="a" name="b" class="c" style="color:red" onclick="x()" onmouseover="y()">hi</p>',
          '<a href="https://ok.test" data-tauri-drag-region aria-describedby="z" id="documentElement" name="cookie" onfocus="x()">link</a>',
          "[md](https://md.test)",
        ].join("\n\n"),
      ),
    );
    const names = new Set<string>();
    for (const el of Array.from(host.querySelectorAll("*"))) {
      for (const name of el.getAttributeNames()) names.add(`${el.nodeName.toLowerCase()}:${name}`);
    }
    expect(Array.from(names).sort()).toEqual(["a:href", "a:rel"]);
    expect(host.querySelector("p")?.textContent).toBe("hi");
  });

  it("empty_input_renders_nothing", () => {
    expect(renderMarkdown("")).toBe("");
  });
});
