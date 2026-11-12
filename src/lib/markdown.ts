import DOMPurify from "dompurify";
import { marked } from "marked";

// Notes are attacker-controlled (a synced or imported vault), so the HTML that `marked` produces
// is never trusted: DOMPurify keeps an allow-list of text-structure tags and one attribute.
const ALLOWED_TAGS = [
  "p", "br", "strong", "em", "ul", "ol", "li", "code", "pre", "blockquote",
  "h1", "h2", "h3", "h4", "h5", "h6", "a", "hr",
];
const SAFE_HREF = /^(https?:|mailto:)/i;

let configured = false;

function configure(): void {
  if (configured) return;
  configured = true;
  DOMPurify.addHook("afterSanitizeAttributes", (node) => {
    if (node.nodeName !== "A") return;
    const href = node.getAttribute("href");
    if (href === null || !SAFE_HREF.test(href.trim())) node.removeAttribute("href");
    node.setAttribute("rel", "noopener noreferrer");
  });
}

/** Markdown to sanitised HTML: no scripts, styles, forms, images or event handlers; links are http(s) or mailto only. */
export function renderMarkdown(src: string): string {
  if (src === "") return "";
  configure();
  const html = marked.parse(src, { async: false, gfm: true });
  return DOMPurify.sanitize(html, { ALLOWED_TAGS, ALLOWED_ATTR: ["href"] });
}
