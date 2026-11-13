<script lang="ts">
  // The attachments of an entry. Export is one backend call: the save dialog opens in Rust, the
  // webview passes only the entry id and the attachment name (R7); `false` is a cancelled dialog.
  import { api } from "../lib/api";
  import type { AttachmentMeta } from "../lib/bindings";
  import { formatBytes } from "../lib/i18n/format";
  import { t } from "../lib/i18n/index.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import Button from "../lib/ui/Button.svelte";
  import Icon from "../lib/ui/Icon.svelte";

  let { entryId, attachments }: { entryId: string; attachments: AttachmentMeta[] } = $props();

  // `true` is a saved file (a short confirmation), `false` a cancelled dialog (silent).
  const exportFile = (name: string) =>
    toasts.guard(async () => {
      if (await api.attachmentExport(entryId, name)) toasts.push("success", t("Exported."));
    });
</script>

<div class="files" role="group" aria-label={t("Attachments")}>
  <h3>{t("Attachments")}</h3>
  <ul>
    {#each attachments as file (file.name)}
      <li>
        <Icon name="paperclip" size={14} />
        <span class="name">{file.name}</span>
        {#if file.size !== null}<span class="size">{formatBytes(file.size)}</span>{/if}
        <Button variant="plain" onclick={() => exportFile(file.name)}>{t("Export…")}</Button>
      </li>
    {/each}
  </ul>
</div>

<style>
  .files {
    margin-top: 16px;
  }
  h3 {
    margin: 0 0 4px;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-2);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 0;
    border-bottom: var(--sep);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .size {
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
</style>
