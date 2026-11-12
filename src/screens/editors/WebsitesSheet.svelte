<script lang="ts">
  // Edits the websites and the match mode of an entry's edit model, in a modal sheet. It works on
  // the model the owner passes (the store-held `navigation.editModel`) and touches nothing else:
  // Save writes the cleaned URL list and the mode into the model and calls `onsave`, Cancel
  // calls `oncancel`; the owner persists or drops the model. The entry editor (Task 9) reuses it.
  import { untrack } from "svelte";
  import type { EntryEditModel_Serialize } from "../../lib/bindings";
  import { t } from "../../lib/i18n/index.svelte";
  import Button from "../../lib/ui/Button.svelte";
  import Modal from "../../lib/ui/Modal.svelte";
  import Select from "../../lib/ui/Select.svelte";

  let {
    model,
    onsave,
    oncancel,
  }: { model: EntryEditModel_Serialize; onsave: () => void | Promise<void>; oncancel: () => void } = $props();

  // Rows are the sheet's own copy until Save, so Cancel needs no undo.
  let rows = $state(untrack(() => (model.urls.length > 0 ? [...model.urls] : [""])));
  let mode = $state(untrack(() => model.uriMatchMode));
  let saving = $state(false);

  const modes = $derived([
    { value: "registrable-domain", label: t("Registrable domain (recommended)") },
    { value: "host", label: t("Exact host") },
    { value: "starts-with", label: t("Starts with") },
    { value: "exact", label: t("Exact address") },
    { value: "never", label: t("Never") },
  ]);

  async function save() {
    if (saving) return;
    saving = true;
    model.urls = rows.map((u) => u.trim()).filter((u) => u !== "");
    model.uriMatchMode = mode;
    try {
      await onsave();
    } finally {
      saving = false;
    }
  }
</script>

<Modal open={true} title={t("Websites")} onclose={oncancel}>
  <div class="rows">
    {#each rows as _, i (i)}
      <div class="row">
        <input class="url" type="text" aria-label={t("URL")} bind:value={rows[i]} autocomplete="off" spellcheck="false" />
        <Button variant="plain" onclick={() => rows.splice(i, 1)}>{t("Remove")}</Button>
      </div>
    {/each}
    <div><Button variant="plain" onclick={() => rows.push("")}>{t("Add Website")}</Button></div>
    <div class="mode">
      <span>{t("Match")}</span>
      <Select value={mode} options={modes} label={t("Match")} onchange={(v) => (mode = v)} />
    </div>
  </div>
  {#snippet footer()}
    <Button onclick={oncancel}>{t("Cancel")}</Button>
    <Button variant="primary" disabled={saving} onclick={save}>{t("Save")}</Button>
  {/snippet}
</Modal>

<style>
  .rows {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 380px;
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .url {
    flex: 1;
    min-width: 0;
    padding: 5px 8px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
    background: var(--control-bg);
    color: var(--text);
    font: inherit;
  }
  .mode {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--text-2);
  }
</style>
