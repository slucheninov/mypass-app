<script lang="ts">
  // Edits the websites and the match mode of an entry's edit model, in a modal sheet. It works on
  // the model the owner passes (the store-held `navigation.editModel`) and touches nothing else:
  // Save writes the cleaned URL list and the mode into the model and calls `onsave`, Cancel
  // calls `oncancel`; the owner persists or drops the model. The entry editor reuses it.
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

  // The five modes in the spec's order, each with a one-line explanation.
  const modes = $derived([
    { value: "registrable-domain", label: t("Same site"), note: t("Offered on every page of the same site, subdomains included") },
    { value: "host", label: t("Same host"), note: t("Offered only on pages of exactly this host name") },
    { value: "starts-with", label: t("Starts with"), note: t("Offered on pages whose address begins with this one") },
    { value: "exact", label: t("Exact URL"), note: t("Offered only on this exact page address") },
    { value: "never", label: t("Never offer"), note: t("Never offered automatically; kept for reference") },
  ]);
  const note = $derived(modes.find((m) => m.value === mode)?.note ?? "");

  function move(i: number, by: -1 | 1) {
    const j = i + by;
    const a = rows[i];
    const b = rows[j];
    if (a === undefined || b === undefined) return;
    rows[i] = b;
    rows[j] = a;
  }

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
        <Button variant="plain" disabled={i === 0} onclick={() => move(i, -1)}>{t("Move up")}</Button>
        <Button variant="plain" disabled={i === rows.length - 1} onclick={() => move(i, 1)}>{t("Move down")}</Button>
        <Button variant="plain" onclick={() => rows.splice(i, 1)}>{t("Remove")}</Button>
      </div>
    {/each}
    <div><Button variant="plain" onclick={() => rows.push("")}>{t("Add Website")}</Button></div>
    <div class="mode">
      <span>{t("Match mode")}</span>
      <Select value={mode} options={modes} label={t("Match mode")} onchange={(v) => (mode = v)} />
    </div>
    <p class="note" data-testid="mode-explanation">{note}</p>
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
  .note {
    margin: 0;
    color: var(--text-2);
    font-size: 12px;
  }
  .mode {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--text-2);
  }
</style>
