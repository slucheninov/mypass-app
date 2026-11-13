<script module lang="ts">
  /** What the editor works on: a new group under `parentId` (`null` is the top level), or an existing one. */
  export type GroupEditorTarget =
    | { mode: "create"; parentId: string | null }
    | { mode: "edit"; id: string; name: string; emoji: string | null };
</script>

<script lang="ts">
  import { untrack } from "svelte";
  import { api, errorMessage } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import Button from "../lib/ui/Button.svelte";
  import EmojiPicker from "../lib/ui/EmojiPicker.svelte";
  import Modal from "../lib/ui/Modal.svelte";

  let { target, onclose }: { target: GroupEditorTarget; onclose: () => void } = $props();

  // The form starts from the target it is opened for; the parent mounts a fresh editor per target.
  let name = $state(untrack(() => (target.mode === "edit" ? target.name : "")));
  let emoji = $state<string | null>(untrack(() => (target.mode === "edit" ? target.emoji : null)));
  let error = $state<string | null>(null);
  let busy = $state(false);

  async function save(event?: Event) {
    event?.preventDefault();
    const trimmed = name.trim();
    if (trimmed === "" || busy) return;
    busy = true;
    error = null;
    try {
      if (target.mode === "edit") await api.groupUpdate(target.id, trimmed, emoji);
      else await api.groupCreate(target.parentId, trimmed, emoji);
      onclose();
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = false;
    }
  }
</script>

<Modal open={true} title={target.mode === "edit" ? t("Edit group…") : t("New Group")} {onclose}>
  <div class="form">
    <label class="field">
      <span>{t("Group name")}</span>
      <input type="text" bind:value={name} autocomplete="off" onkeydown={(e) => e.key === "Enter" && !e.isComposing && void save(e)} />
    </label>
    <div class="field">
      <span>{t("Icon")}</span>
      <EmojiPicker value={emoji} onchange={(v) => (emoji = v)} />
    </div>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  </div>
  {#snippet footer()}
    <Button onclick={onclose}>{t("Cancel")}</Button>
    <Button variant="primary" disabled={name.trim() === "" || busy} onclick={() => void save()}>{t("Save")}</Button>
  {/snippet}
</Modal>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 320px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--text-2);
  }
  input[type="text"] {
    padding: 6px 8px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
    background: var(--control-bg);
    color: var(--text);
    font-size: 13px;
  }
  .error {
    margin: 0;
    color: var(--danger);
    font-size: 12px;
  }
</style>
