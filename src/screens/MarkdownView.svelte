<script lang="ts">
  // The one place where HTML is injected: `renderMarkdown` output is already sanitised (an
  // allow-list of text tags, `href` only, no script, style, form or image). Links never navigate
  // the webview; http(s) goes through the opener, everything else does nothing.
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { renderMarkdown } from "../lib/markdown";
  import { webUrl } from "../lib/web-url";
  import { toasts } from "../lib/stores/toasts.svelte";

  let { source }: { source: string } = $props();

  const html = $derived(renderMarkdown(source));
  let root: HTMLElement | undefined = $state();

  // A listener on the container (not a handler attribute) so the generated links keep working
  // and the element stays a plain, non-interactive block for the accessibility checks.
  $effect(() => {
    const el = root;
    if (!el) return;
    const onclick = (event: MouseEvent) => {
      const anchor = event.target instanceof Element ? event.target.closest("a") : null;
      if (!anchor || !el.contains(anchor)) return;
      event.preventDefault();
      const url = webUrl(anchor.getAttribute("href") ?? "");
      if (url !== null) void openUrl(url).catch((e) => toasts.reportError(e));
    };
    el.addEventListener("click", onclick);
    return () => el.removeEventListener("click", onclick);
  });
</script>

<div class="markdown" bind:this={root}>{@html html}</div>

<style>
  .markdown {
    line-height: 1.5;
    overflow-wrap: anywhere;
    user-select: text;
    -webkit-user-select: text;
  }
  .markdown :global(p),
  .markdown :global(ul),
  .markdown :global(ol),
  .markdown :global(pre),
  .markdown :global(blockquote) {
    margin: 0 0 8px;
  }
  .markdown :global(code) {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    background: var(--hover);
    padding: 0 3px;
    border-radius: 3px;
  }
  .markdown :global(pre) {
    background: var(--hover);
    padding: 8px;
    border-radius: var(--radius);
    overflow: auto;
  }
  .markdown :global(blockquote) {
    border-left: 3px solid var(--separator);
    padding-left: 10px;
    color: var(--text-2);
  }
  .markdown :global(a) {
    color: var(--accent);
    cursor: pointer;
  }
  .markdown :global(a:not([href])) {
    color: inherit;
    cursor: text;
  }
</style>
