<script lang="ts">
  // The user-defined fields of an entry: name, value, a protected switch and Remove per row.
  // A protected value is drawn as a password input; the rows are edited in place through `fields`.
  import type { CustomFieldEdit } from "../../lib/bindings";
  import { t } from "../../lib/i18n/index.svelte";
  import Button from "../../lib/ui/Button.svelte";
  import Toggle from "../../lib/ui/Toggle.svelte";

  let { fields = $bindable() }: { fields: CustomFieldEdit[] } = $props();
</script>

<section class="custom" aria-label={t("Custom fields")}>
  <h3>{t("Custom fields")}</h3>
  {#each fields as field, i (i)}
    <div class="row">
      <input type="text" aria-label={t("Field name")} bind:value={field.key} autocomplete="off" spellcheck="false" />
      <input
        type={field.protected ? "password" : "text"}
        aria-label={t("Field value")}
        bind:value={field.value}
        autocomplete="off"
        spellcheck="false"
      />
      <Toggle checked={field.protected} label={t("Protected")} onchange={(v) => (field.protected = v)} />
      <Button variant="plain" onclick={() => fields.splice(i, 1)}>{t("Remove")}</Button>
    </div>
  {/each}
  <div><Button variant="plain" onclick={() => fields.push({ key: "", value: "", protected: false })}>{t("Add field")}</Button></div>
</section>

<style>
  .custom {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  h3 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-2);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  input {
    flex: 1;
    min-width: 0;
    padding: 5px 8px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
    background: var(--control-bg);
    color: var(--text);
    font: inherit;
  }
</style>
