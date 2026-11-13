<script lang="ts">
  // One labelled row of the tray's compact detail: the value, an optional Reveal toggle for a
  // secret and an optional Copy. A revealed value lives in the `Revealed` the owner passes in; the
  // copy goes through the backend, so the value never has to be revealed to be copied.
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

<div class="row" role="group" aria-label={label}>
  <div class="label">{label}</div>
  <div class="line">
    <div class="value" class:mono={shown !== null}>
      {#if children}{@render children()}{:else if secret}{shown ?? "••••••••"}{:else}{text}{/if}
    </div>
    {#if secret && field}
      <IconButton
        icon={shown === null ? "eye" : "eye-off"}
        label={shown === null ? t("Reveal") : t("Hide")}
        onclick={toggle}
      />
    {/if}
    {#if field}<IconButton icon="copy" label={t("Copy")} onclick={copy} />{/if}
  </div>
</div>

<style>
  .row {
    padding: 6px 0;
    border-bottom: var(--sep);
  }
  .label {
    color: var(--text-2);
    font-size: 11px;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .value {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
    user-select: text;
  }
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  }
</style>
