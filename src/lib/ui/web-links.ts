// One delegated handler for every link that vault data can put on screen (rendered markdown, the
// websites of a login). Such a link must never navigate the webview or open a window: every way
// to activate it is intercepted here, http(s) goes through the opener (whose capability scope is
// http and https only) and everything else does nothing. The Rust side refuses navigation outside
// the app origin as a second layer (`navigation.rs`).
import { openUrl } from "@tauri-apps/plugin-opener";
import { toasts } from "../stores/toasts.svelte";
import { webUrl } from "../web-url";

function anchorOf(event: Event, root: HTMLElement): HTMLAnchorElement | null {
  const found = event.target instanceof Element ? event.target.closest("a") : null;
  return found && root.contains(found) ? found : null;
}

function open(anchor: HTMLAnchorElement): void {
  const url = webUrl(anchor.getAttribute("href") ?? "");
  if (url !== null) void openUrl(url).catch((e) => toasts.reportError(e));
}

/** Svelte action: `use:webLinks` on the container of the links. */
export function webLinks(root: HTMLElement): { destroy(): void } {
  const onclick = (event: MouseEvent) => {
    const anchor = anchorOf(event, root);
    if (!anchor) return;
    event.preventDefault();
    open(anchor);
  };
  // A middle click (or any other non-primary button) would otherwise open a new window.
  const onauxclick = (event: MouseEvent) => {
    const anchor = anchorOf(event, root);
    if (!anchor) return;
    event.preventDefault();
    if (event.button === 1) open(anchor);
  };
  // Enter on a focused link activates it; cancelling the keydown cancels that activation.
  const onkeydown = (event: KeyboardEvent) => {
    if (event.key !== "Enter") return;
    const anchor = anchorOf(event, root);
    if (!anchor) return;
    event.preventDefault();
    open(anchor);
  };
  // "Open in new window" of the context menu and dragging a link onto a webview.
  const cancel = (event: Event) => {
    if (anchorOf(event, root)) event.preventDefault();
  };
  root.addEventListener("click", onclick);
  root.addEventListener("auxclick", onauxclick);
  root.addEventListener("keydown", onkeydown);
  root.addEventListener("contextmenu", cancel);
  root.addEventListener("dragstart", cancel);
  return {
    destroy() {
      root.removeEventListener("click", onclick);
      root.removeEventListener("auxclick", onauxclick);
      root.removeEventListener("keydown", onkeydown);
      root.removeEventListener("contextmenu", cancel);
      root.removeEventListener("dragstart", cancel);
    },
  };
}
