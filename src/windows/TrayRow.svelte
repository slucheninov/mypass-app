<script lang="ts">
  // One row of the tray's compact detail: the label on the left, the value on the right. A row
  // with a backend field copies it on click (the value never has to be revealed to be copied); a
  // secret is masked and has its own Reveal toggle. A revealed value lives in the `Revealed` the
  // owner passes in.
  import type { Snippet } from "svelte";
  import { api } from "../lib/api";
  import type { FieldRef } from "../lib/bindings";
  import { t } from "../lib/i18n/index.svelte";
  import type { Revealed } from "../lib/secret.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import IconButton from "../lib/ui/IconButton.svelte";
  import { copyField } from "../screens/detail-actions";

  let {
    label,
    entryId,
    field = null,
    text = "",
    secret = null,
    children,
  }: {
    label: string;
    entryId: string;
    /** The backend reference to copy (and reveal); `null` for a row that only shows a value. */
    field?: FieldRef | null;
    text?: string;
    /** Present for a secret: masked until revealed. */
    secret?: Revealed | null;
    children?: Snippet;
  } = $props();

  const shown = $derived(secret ? secret.value : null);

  const toggle = () =>
    toasts.guard(async () => {
      if (!secret || !field) return;
      if (secret.value !== null) secret.hide();
      else await secret.reveal(() => api.entryReveal(entryId, field));
    });
  const copy = () =>
    toasts.guard(async () => {
      if (field) await copyField(entryId, field);
    });
</script>

{#snippet value()}
  <span class="value" class:mono={shown !== null} class:masked={secret !== null && shown === null}>
    {#if children}{@render children()}{:else if secret}{shown ?? "••••••••"}{:else}{text}{/if}
  </span>
{/snippet}

<div class="row" role="group" aria-label={label}>
  <span class="label">{label}</span>
  {#if field}
    <!-- The whole value is the copy target, like a native detail row. -->
    <button type="button" class="copy" aria-label={t("Copy")} title={t("Copy")} onclick={copy}>
      {@render value()}
    </button>
  {:else}
    {@render value()}
  {/if}
  {#if secret && field}
    <IconButton
      icon={shown === null ? "eye" : "eye-off"}
      label={shown === null ? t("Reveal") : t("Hide")}
      onclick={toggle}
    />
  {/if}
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 40px;
    padding: 0 12px;
  }
  .label {
    flex: none;
    max-width: 45%;
    font-size: var(--font-size-md);
  }
  .copy {
    flex: 1;
    min-width: 0;
    display: flex;
    justify-content: flex-end;
    margin: 4px -6px 4px 0;
    padding: 4px 6px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: right;
    cursor: default;
  }
  .copy:hover,
  .copy:focus-visible {
    background: var(--hover);
    outline: none;
  }
  .copy:focus-visible {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .value {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
    text-align: right;
    font-size: var(--font-size-md);
    user-select: text;
  }
  .row > .value {
    color: var(--text-2);
  }
  .masked {
    letter-spacing: 0.08em;
  }
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  }
</style>
