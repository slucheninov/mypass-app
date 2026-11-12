<script lang="ts">
  // The CSV import dialog. The file is chosen in a native dialog that Rust opens (the webview never
  // names a path): `import_csv_preview` answers with the counts and the first titles only (no
  // password ever reaches this component) or `null` when the user cancelled, and
  // `import_csv_commit(groupId)` then imports the file that preview remembered, exactly once.
  import { onMount } from "svelte";
  import { api, ApiFailure, errorMessage } from "../lib/api";
  import type { CsvPreview, SkipReason } from "../lib/bindings";
  import { visibleGroups } from "../lib/group-tree";
  import { t } from "../lib/i18n/index.svelte";
  import { lockGuard } from "../lib/lock-guard";
  import { entries } from "../lib/stores/entries.svelte";
  import { navigation } from "../lib/stores/navigation.svelte";
  import { session } from "../lib/stores/session.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import Button from "../lib/ui/Button.svelte";
  import Modal from "../lib/ui/Modal.svelte";
  import Select from "../lib/ui/Select.svelte";

  let { onclose }: { onclose: () => void } = $props();

  let preview = $state<CsvPreview | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let groupId = $state<string | null>(null);
  let closed = false;

  const rootId = $derived(entries.sidebar?.rootGroupId ?? null);
  const options = $derived.by(() => {
    const sidebar = entries.sidebar;
    if (!sidebar) return [];
    const rows = visibleGroups(sidebar.groups, {}).map(({ node, level }) => ({
      value: node.id,
      label: `${"  ".repeat(level)}${node.name}`,
    }));
    return [{ value: sidebar.rootGroupId, label: t("Vault") }, ...rows];
  });
  // The current group when browsing one, else the top level; a group that is not offered (the bin)
  // falls back to the top level.
  const chosen = $derived.by(() => {
    const wanted = groupId ?? (navigation.scope.kind === "group" ? navigation.scope.groupId : rootId);
    return options.some((o) => o.value === wanted) ? wanted : rootId;
  });

  /**
   * The backend reports a bad file as `invalidInput` with a fixed English message (the code has no
   * finer grain, R15); those three messages are pinned by a Rust test, anything else falls back to
   * the code's own localised text.
   */
  function failureMessage(e: unknown): string {
    if (e instanceof ApiFailure && e.code === "invalidInput") {
      if (e.detail.startsWith("missing required column")) return t("The file must have title, username and password columns");
      if (e.detail === "file is not valid UTF-8") return t("The file is not UTF-8 text");
      if (e.detail === "file is empty") return t("The file is empty");
    }
    return errorMessage(e);
  }

  const reason = (line: number, code: SkipReason): string =>
    code === "fieldCountMismatch"
      ? t("Row %lld: the number of columns does not match", line)
      : t("Row %lld: a quoted value is not closed", line);

  function close() {
    closed = true;
    onclose();
  }

  async function choose(): Promise<void> {
    const unchanged = lockGuard();
    error = null;
    preview = null;
    try {
      const picked = await api.importCsvPreview();
      if (closed || !unchanged()) return;
      if (picked === null) close();
      else preview = picked;
    } catch (e) {
      if (closed || !unchanged()) return;
      error = failureMessage(e);
    }
  }

  async function commit(): Promise<void> {
    if (!preview || chosen === null || busy) return;
    busy = true;
    error = null;
    const unchanged = lockGuard();
    try {
      const result = await api.importCsvCommit(chosen);
      if (!unchanged()) return;
      toasts.push("success", t("%lld items imported", result.imported));
      void entries.refresh();
      close();
    } catch (e) {
      if (unchanged()) error = failureMessage(e);
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    void choose();
    return () => {
      closed = true;
    };
  });

  // A lock takes the dialog (and the remembered file's preview) away.
  $effect(() => {
    if (session.status.state !== "unlocked") close();
  });
</script>

<Modal open={true} title={t("Import CSV")} onclose={close}>
  <div class="body">
    {#if preview}
      <p class="count">{t("%lld items found", preview.total)}</p>
      <ul class="titles">
        {#each preview.sampleTitles as title, index (index)}<li>{title}</li>{/each}
      </ul>
      {#if preview.skipped.length > 0}
        <ul class="skipped">
          {#each preview.skipped as row, index (index)}<li>{reason(row.line, row.reason)}</li>{/each}
        </ul>
      {/if}
      <div class="group">
        <span>{t("Import into")}</span>
        {#if chosen !== null}
          <Select value={chosen} {options} label={t("Import into")} onchange={(id) => (groupId = id)} />
        {/if}
      </div>
    {:else if error === null}
      <p class="hint">{t("Choose a CSV file")}</p>
    {/if}
    {#if error}
      <p class="error" role="alert">{error}</p>
      {#if !preview}
        <Button onclick={() => void choose()}>{t("Choose a CSV file")}</Button>
      {/if}
    {/if}
  </div>
  {#snippet footer()}
    <Button onclick={close}>{t("Cancel")}</Button>
    {#if preview}
      <Button variant="primary" disabled={busy || chosen === null} onclick={() => void commit()}>{t("Import")}</Button>
    {/if}
  {/snippet}
</Modal>

<style>
  .body {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 320px;
    max-width: 420px;
  }
  .count {
    margin: 0;
    font-weight: 600;
  }
  ul {
    margin: 0;
    padding-left: 18px;
  }
  .skipped {
    color: var(--text-2);
  }
  .group {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .group > span {
    color: var(--text-2);
  }
  .hint {
    margin: 0;
    color: var(--text-2);
  }
  .error {
    margin: 0;
    color: var(--danger);
  }
</style>
