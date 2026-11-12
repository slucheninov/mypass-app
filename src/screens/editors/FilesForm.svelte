<script lang="ts">
  // Attachments of a Files item. "Add files..." opens the native dialog from Rust
  // (`attachment_pick`), which hands back single-use tokens, never paths; each token becomes an
  // `{ kind: "add", path: token }` op that the save swaps for the real file. Removing an existing
  // file queues `{ kind: "remove", name }`; nothing reaches the vault until the editor saves.
  import { api } from "../../lib/api";
  import type { EntryEditModel_Serialize, PickedFile } from "../../lib/bindings";
  import { formatBytes } from "../../lib/i18n/format";
  import { t } from "../../lib/i18n/index.svelte";
  import { lockGuard } from "../../lib/lock-guard";
  import { toasts } from "../../lib/stores/toasts.svelte";
  import Button from "../../lib/ui/Button.svelte";

  let {
    model = $bindable(),
    error = null,
  }: {
    model: EntryEditModel_Serialize;
    /** The localised message of a rejected save (`limitExceeded`), shown under the list. */
    error?: string | null;
  } = $props();

  /** The files picked in this editor, by token (the names and sizes come from Rust). */
  let picked = $state<PickedFile[]>([]);

  const removed = $derived(new Set(model.attachmentOps.flatMap((op) => (op.kind === "remove" ? [op.name] : []))));
  const existing = $derived(model.attachments.filter((a) => !removed.has(a.name)));

  async function add() {
    const unchanged = lockGuard();
    await toasts.guard(async () => {
      const files = await api.attachmentPick();
      if (!unchanged()) return;
      for (const file of files) {
        picked.push(file);
        model.attachmentOps.push({ kind: "add", path: file.token });
      }
    });
  }

  function removeExisting(name: string) {
    model.attachmentOps.push({ kind: "remove", name });
  }

  function removeQueued(token: string) {
    picked = picked.filter((f) => f.token !== token);
    model.attachmentOps = model.attachmentOps.filter((op) => !(op.kind === "add" && op.path === token));
  }

  const size = (bytes: number | null) => (bytes === null ? "" : formatBytes(bytes));
</script>

<div class="files">
  <div><Button onclick={add}>{t("Add files…")}</Button></div>
  {#if existing.length + picked.length > 0}
    <ul aria-label={t("Attachments")}>
      {#each existing as file (file.name)}
        <li>
          <span class="name">{file.name}</span>
          <span class="size">{size(file.size)}</span>
          <Button variant="plain" onclick={() => removeExisting(file.name)}>{t("Remove")}</Button>
        </li>
      {/each}
      {#each picked as file (file.token)}
        <li>
          <span class="name">{file.name}</span>
          <span class="size">{size(file.size)}</span>
          <Button variant="plain" onclick={() => removeQueued(file.token)}>{t("Remove")}</Button>
        </li>
      {/each}
    </ul>
  {/if}
  {#if error}<p class="error" role="alert" tabindex="-1">{error}</p>{/if}
  <p class="hint">{t("8 files per item, 5 MB each")}</p>
</div>

<style>
  .files {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 0;
    border-bottom: var(--sep);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .size,
  .hint {
    color: var(--text-2);
    font-size: 12px;
  }
  .hint,
  .error {
    margin: 0;
  }
  .error {
    color: var(--danger);
  }
</style>
