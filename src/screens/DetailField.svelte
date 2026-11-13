<script lang="ts">
  // One labelled row of the detail pane: the value (plain text, a masked secret or custom
  // markup), Reveal/Hide and Copy buttons, and a context menu (right click, the menu key or
  // Shift+F10 from a button inside the row) with Show Large Type, Copy and Reveal. Nothing here
  // holds a secret: a revealed value lives in the `Revealed` the owner passes in.
  import type { Snippet } from "svelte";
  import { api } from "../lib/api";
  import type { FieldRef } from "../lib/bindings";
  import { t } from "../lib/i18n/index.svelte";
  import type { Revealed } from "../lib/secret.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import ContextMenu from "../lib/ui/ContextMenu.svelte";
  import FieldRow from "../lib/ui/FieldRow.svelte";
  import IconButton from "../lib/ui/IconButton.svelte";
  import type { MenuItem } from "../lib/ui/menu";
  import { copyField } from "./detail-actions";

  type Props = {
    label: string;
    entryId: string;
    /** The backend reference used to reveal, copy and enlarge the value. */
    field: FieldRef;
    /** Present for a secret: the value is masked until revealed. */
    secret?: Revealed | null;
    /** The visible value of a non-secret row. */
    text?: string;
    /** What stands in for a hidden secret (for example `•••• 1111`). */
    mask?: string;
    /** Large Type needs the value revealed first (card number, security code). */
    largeTypeAfterReveal?: boolean;
    copyable?: boolean;
    onlargetype: (field: FieldRef) => void;
    /** Custom value markup (links, a one-time code); replaces `text` and the mask. */
    children?: Snippet;
    /** Extra buttons after Copy. */
    extra?: Snippet;
  };
  let {
    label,
    entryId,
    field,
    secret = null,
    text = "",
    mask = "••••••••",
    largeTypeAfterReveal = false,
    copyable = true,
    onlargetype,
    children,
    extra,
  }: Props = $props();

  let menu = $state<{ x: number; y: number } | null>(null);
  const shown = $derived(secret ? secret.value : null);

  const toggleReveal = () =>
    toasts.guard(async () => {
      if (!secret) return;
      if (secret.value !== null) secret.hide();
      else await secret.reveal(() => api.entryReveal(entryId, field));
    });
  const copy = () => toasts.guard(() => copyField(entryId, field));

  const items = $derived.by((): MenuItem[] => {
    const list: MenuItem[] = [
      { id: "large", label: t("Show Large Type"), disabled: largeTypeAfterReveal && shown === null },
    ];
    if (copyable) list.push({ id: "copy", label: t("Copy") });
    if (secret) list.push({ id: "reveal", label: shown === null ? t("Reveal") : t("Hide") });
    return list;
  });

  function onselect(id: string) {
    if (id === "large") onlargetype(field);
    else if (id === "copy") void copy();
    else if (id === "reveal") void toggleReveal();
  }

  function oncontextmenu(event: MouseEvent) {
    event.preventDefault();
    menu = { x: event.clientX, y: event.clientY };
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10")) return;
    event.preventDefault();
    const rect = (event.target instanceof Element ? event.target : (event.currentTarget as Element)).getBoundingClientRect();
    menu = { x: rect.left, y: rect.bottom };
  }
</script>

<!-- The menu is an extra path to actions the buttons already offer, so the row itself is not a widget. -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_no_static_element_interactions -->
<div class="line" role="group" aria-label={label} {oncontextmenu} {onkeydown}>
  <FieldRow {label}>
    {#if children}
      {@render children()}
    {:else if secret}
      <span class="secret" class:mono={shown !== null}>{shown ?? mask}</span>
    {:else}
      {text}
    {/if}
    {#snippet actions()}
      {#if secret}
        <IconButton
          icon={shown === null ? "eye" : "eye-off"}
          label={shown === null ? t("Reveal") : t("Hide")}
          onclick={toggleReveal}
        />
      {/if}
      {#if copyable}<IconButton icon="copy" label={t("Copy")} onclick={copy} />{/if}
      {@render extra?.()}
    {/snippet}
  </FieldRow>
</div>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} {items} {onselect} onclose={() => (menu = null)} />
{/if}

<style>
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  }
</style>
