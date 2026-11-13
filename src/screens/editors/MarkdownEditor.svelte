<script lang="ts">
  // A textarea with a small Markdown toolbar and a Preview toggle. Programmatic edits (toolbar,
  // undo, redo) replace the textarea value, which breaks the browser's native history, so the
  // editor keeps its own undo stack (src/lib/undo.ts) and handles Mod+Z, Mod+Shift+Z and Mod+Y.
  // The stack holds note text: it is cleared when the editor is destroyed (close, save, discard)
  // and whenever `session.lockEpoch` changes, and it lives only in this component.
  import { onDestroy, tick } from "svelte";
  import { t } from "../../lib/i18n/index.svelte";
  import { session } from "../../lib/stores/session.svelte";
  import { createUndoStack, type Selection, type UndoSnapshot } from "../../lib/undo";
  import Button from "../../lib/ui/Button.svelte";
  import MarkdownView from "../MarkdownView.svelte";

  let {
    value = $bindable(),
    label,
    oninput,
  }: {
    value: string;
    /** The accessible name of the textarea, already translated. */
    label: string;
    /** Called after every change of `value` made here (typing, toolbar, undo, redo). */
    oninput?: () => void;
  } = $props();

  const stack = createUndoStack();
  const epoch = session.lockEpoch;
  let area: HTMLTextAreaElement | undefined = $state();
  let previewing = $state(false);
  let canUndo = $state(false);
  let canRedo = $state(false);

  // The base state: what the editor opened with, so the first edit can be undone.
  stack.push(value, { start: 0, end: 0 }, { step: true });

  $effect(() => {
    if (session.lockEpoch !== epoch) {
      stack.clear();
      refresh();
    }
  });

  onDestroy(() => stack.clear());

  function refresh() {
    canUndo = stack.canUndo();
    canRedo = stack.canRedo();
  }

  function selection(): Selection {
    return { start: area?.selectionStart ?? 0, end: area?.selectionEnd ?? 0 };
  }

  async function place(sel: Selection) {
    await tick();
    area?.focus();
    area?.setSelectionRange(sel.start, sel.end);
  }

  function typed() {
    if (!area) return;
    value = area.value;
    stack.push(value, selection());
    refresh();
    oninput?.();
  }

  /** One toolbar edit: the old state keeps its selection, the new one is its own undo step. */
  function edit(next: string, sel: Selection) {
    stack.push(value, selection(), { step: true });
    value = next;
    stack.push(next, sel, { step: true });
    refresh();
    oninput?.();
    void place(sel);
  }

  function apply(snapshot: UndoSnapshot | null) {
    if (!snapshot) return;
    value = snapshot.text;
    refresh();
    oninput?.();
    void place(snapshot.selection);
  }

  /** Replaces the whole text as one undo step (restoring a draft). */
  export function replace(next: string) {
    edit(next, { start: next.length, end: next.length });
  }

  const undo = () => apply(stack.undo());
  const redo = () => apply(stack.redo());

  function wrap(before: string, after = before) {
    const { start, end } = selection();
    const inner = value.slice(start, end);
    edit(value.slice(0, start) + before + inner + after + value.slice(end), {
      start: start + before.length,
      end: start + before.length + inner.length,
    });
  }

  function link() {
    const { start, end } = selection();
    const inner = value.slice(start, end);
    // No host after the scheme: a placeholder, never a real address.
    const target = "https://";
    const text = `[${inner}](${target})`;
    const from = start + inner.length + 3;
    edit(value.slice(0, start) + text + value.slice(end), { start: from, end: from + target.length });
  }

  /** Prefixes every line the selection touches. */
  function prefixLines(prefix: string) {
    const { start, end } = selection();
    const lineStart = value.lastIndexOf("\n", start - 1) + 1;
    const block = value.slice(lineStart, end);
    const prefixed = block
      .split("\n")
      .map((line) => prefix + line)
      .join("\n");
    edit(value.slice(0, lineStart) + prefixed + value.slice(end), {
      start: start + prefix.length,
      end: end + (prefixed.length - block.length),
    });
  }

  // The Edit menu and the context menu fire the browser's own history, which does not know the
  // programmatic edits above; they are routed to the same stack instead.
  function onbeforeinput(event: InputEvent) {
    if (event.inputType === "historyUndo") {
      event.preventDefault();
      undo();
    } else if (event.inputType === "historyRedo") {
      event.preventDefault();
      redo();
    }
  }

  function onkeydown(event: KeyboardEvent) {
    if (!(event.metaKey || event.ctrlKey) || event.altKey) return;
    const key = event.key.toLowerCase();
    if (key === "z") {
      event.preventDefault();
      if (event.shiftKey) redo();
      else undo();
    } else if (key === "y" && !event.shiftKey) {
      event.preventDefault();
      redo();
    }
  }
</script>

<div class="md-editor">
  <div class="toolbar" role="toolbar" aria-label={t("Formatting")}>
    <Button variant="plain" disabled={!canUndo || previewing} onclick={undo}>{t("Undo")}</Button>
    <Button variant="plain" disabled={!canRedo || previewing} onclick={redo}>{t("Redo")}</Button>
    <Button variant="plain" disabled={previewing} onclick={() => wrap("**")}>{t("Bold")}</Button>
    <Button variant="plain" disabled={previewing} onclick={() => wrap("*")}>{t("Italic")}</Button>
    <Button variant="plain" disabled={previewing} onclick={() => prefixLines("# ")}>{t("Heading")}</Button>
    <Button variant="plain" disabled={previewing} onclick={() => prefixLines("- ")}>{t("List")}</Button>
    <Button variant="plain" disabled={previewing} onclick={link}>{t("Link")}</Button>
    <Button variant="plain" disabled={previewing} onclick={() => wrap("`")}>{t("Code")}</Button>
    <Button variant="plain" aria-pressed={previewing} onclick={() => (previewing = !previewing)}>{t("Preview")}</Button>
  </div>
  <textarea
    bind:this={area}
    aria-label={label}
    {value}
    hidden={previewing}
    rows="10"
    spellcheck="true"
    oninput={typed}
    {onkeydown}
    {onbeforeinput}
  ></textarea>
  {#if previewing}<div class="preview"><MarkdownView source={value} /></div>{/if}
</div>

<style>
  .md-editor {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    gap: 2px;
  }
  textarea {
    width: 100%;
    min-height: 180px;
    padding: 8px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
    background: var(--control-bg);
    color: var(--text);
    font: inherit;
    resize: vertical;
    box-sizing: border-box;
  }
  .preview {
    min-height: 180px;
    padding: 8px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
  }
</style>
