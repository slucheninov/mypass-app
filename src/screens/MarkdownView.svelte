<script lang="ts">
  // The one place where HTML is injected: `renderMarkdown` output is already sanitised (an
  // allow-list of text tags, `href` only, no script, style, form or image). Links never navigate
  // the webview (`webLinks` intercepts every activation); http(s) goes through the opener,
  // everything else does nothing.
  import { renderMarkdown } from "../lib/markdown";
  import { webLinks } from "../lib/ui/web-links";

  let { source }: { source: string } = $props();

  const html = $derived(renderMarkdown(source));
</script>

<div class="markdown" use:webLinks>{@html html}</div>

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
