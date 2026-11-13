<script lang="ts">
  // A payment card: holder, expiry, the number as `•••• 1111` and the security code, both
  // revealed only on request through `entry_reveal` into the entry's reveal group.
  import { onDestroy, untrack } from "svelte";
  import type { CardDto, FieldRef } from "../lib/bindings";
  import { t } from "../lib/i18n/index.svelte";
  import type { RevealGroup } from "../lib/secret.svelte";
  import DetailField from "./DetailField.svelte";
  import FieldRow from "../lib/ui/FieldRow.svelte";

  let {
    entryId,
    card,
    group,
    onlargetype,
  }: { entryId: string; card: CardDto; group: RevealGroup; onlargetype: (field: FieldRef) => void } = $props();

  // A member belongs to this mount; the owner never swaps the group under a mounted row.
  const number = untrack(() => group.create());
  const cvv = untrack(() => group.create());
  onDestroy(() => number.hide());
  onDestroy(() => cvv.hide());
</script>

{#if card.holder !== ""}
  <div role="group" aria-label={t("Cardholder")}>
    <FieldRow label={t("Cardholder")}>{card.holder}</FieldRow>
  </div>
{/if}
{#if card.numberLast4 !== ""}
  <DetailField
    label={t("Card number")}
    {entryId}
    field={{ kind: "cardNumber" }}
    secret={number}
    mask={`•••• ${card.numberLast4}`}
    largeTypeAfterReveal
    {onlargetype}
  />
{/if}
{#if card.expiry !== ""}
  <div role="group" aria-label={t("Expires")}>
    <FieldRow label={t("Expires")}>{card.expiry}</FieldRow>
  </div>
{/if}
{#if card.hasCvv}
  <DetailField
    label={t("Security code")}
    {entryId}
    field={{ kind: "cardCvv" }}
    secret={cvv}
    largeTypeAfterReveal
    {onlargetype}
  />
{/if}
