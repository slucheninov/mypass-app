<script lang="ts">
  // The note body of a Notes item: the Markdown editor plus its drafts. For a saved entry (R30:
  // an unsaved one has no id to key a draft by) the text is sent to `draft_save` 3 s after the last
  // keystroke; on open, saved versions are offered in a banner (Restore draft / Dismiss) and in a
  // versions sheet. Drafts hold the note text and nothing else. The versions are component-local:
  // they are dropped when the editor closes and whenever `session.lockEpoch` changes, and the
  // editor clears them on the backend after a successful save.
  import { onDestroy, onMount } from "svelte";
  import { api } from "../../lib/api";
  import type { DraftVersionDto, EntryEditModel_Serialize } from "../../lib/bindings";
  import { formatDate } from "../../lib/i18n/format";
  import { t } from "../../lib/i18n/index.svelte";
  import { lockGuard } from "../../lib/lock-guard";
  import { session } from "../../lib/stores/session.svelte";
  import Button from "../../lib/ui/Button.svelte";
  import Modal from "../../lib/ui/Modal.svelte";
  import MarkdownEditor from "./MarkdownEditor.svelte";

  /** Idle time after the last keystroke before a draft is written. */
  const DRAFT_IDLE_MS = 3000;

  let { model = $bindable() }: { model: EntryEditModel_Serialize } = $props();

  // The entry is fixed for this form's lifetime (the editor is keyed by its model).
  // svelte-ignore state_referenced_locally
  const entryId = model.id;
  // svelte-ignore state_referenced_locally
  const baseline = model.notes;
  const epoch = session.lockEpoch;

  let editor: { replace(text: string): void } | undefined = $state();
  let versions = $state<DraftVersionDto[]>([]);
  let banner = $state(false);
  let sheet = $state(false);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let closed = false;

  const newestFirst = (a: DraftVersionDto, b: DraftVersionDto) => (b.savedAt ?? 0) - (a.savedAt ?? 0);

  onMount(() => {
    if (entryId === null) return;
    const unchanged = lockGuard();
    void api
      .draftVersions(entryId)
      .then((found) => {
        if (closed || !unchanged()) return;
        versions = [...found].sort(newestFirst);
        banner = versions.length > 0;
      })
      // Drafts are a convenience: a failure means "no drafts", never an error in the editor.
      .catch(() => undefined);
  });

  function forget() {
    closed = true;
    if (timer !== null) clearTimeout(timer);
    timer = null;
    versions = [];
    banner = false;
    sheet = false;
  }

  $effect(() => {
    if (session.lockEpoch !== epoch) forget();
  });
  onDestroy(forget);

  /** Called after every edit of the note text. */
  function changed() {
    if (entryId === null || closed) return;
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => void writeDraft(entryId), DRAFT_IDLE_MS);
  }

  async function writeDraft(id: string) {
    timer = null;
    if (closed || model.notes === baseline) return;
    try {
      const stored = await api.draftSave(id, model.notes);
      if (!stored && !closed) {
        // No draft key: the editor works without drafts.
        versions = [];
        banner = false;
        sheet = false;
      }
    } catch {
      // Best effort; the note itself is saved by the editor's Save.
    }
  }

  function restore(version: DraftVersionDto) {
    editor?.replace(version.notes);
    banner = false;
    sheet = false;
  }

  async function dismiss() {
    banner = false;
    versions = [];
    sheet = false;
    if (entryId === null) return;
    try {
      await api.draftClear(entryId);
    } catch {
      // Nothing to show: the drafts expire by themselves.
    }
  }

  const preview = (text: string) => {
    const line = text.split("\n").find((l) => l.trim() !== "") ?? "";
    return line.length > 80 ? `${line.slice(0, 80)}…` : line;
  };
</script>

<div class="notes-form">
  {#if banner && versions[0]}
    {@const newest = versions[0]}
    <section class="banner" aria-label={t("Draft found")}>
      <span>{t("Draft found")}</span>
      <Button onclick={() => restore(newest)}>{t("Restore draft")}</Button>
      <Button variant="plain" onclick={() => (sheet = true)}>{t("Drafts")}</Button>
      <Button variant="plain" onclick={dismiss}>{t("Dismiss")}</Button>
    </section>
  {/if}
  <MarkdownEditor bind:this={editor} bind:value={model.notes} label={t("Notes")} oninput={changed} />
</div>

{#if sheet}
  <Modal open={true} title={t("Drafts")} onclose={() => (sheet = false)}>
    <ul class="versions">
      {#each versions as version (version.id)}
        <li>
          <span class="when">{version.savedAt === null ? "" : formatDate(version.savedAt, "datetime")}</span>
          <span class="preview">{preview(version.notes)}</span>
          <Button onclick={() => restore(version)}>{t("Restore draft")}</Button>
        </li>
      {/each}
    </ul>
    {#snippet footer()}
      <Button onclick={() => (sheet = false)}>{t("Close")}</Button>
    {/snippet}
  </Modal>
{/if}

<style>
  .notes-form {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
    background: var(--hover);
  }
  .banner span {
    flex: 1;
  }
  .versions {
    margin: 0;
    padding: 0;
    list-style: none;
    min-width: 380px;
  }
  .versions li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 0;
    border-bottom: var(--sep);
  }
  .when {
    flex: none;
    color: var(--text-2);
    font-size: 12px;
  }
  .preview {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
