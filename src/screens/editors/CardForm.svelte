<script lang="ts">
  // The four card fields. The number and the security code are drawn as password inputs, so they
  // are not readable over a shoulder while the card is edited.
  import type { CardEdit } from "../../lib/bindings";
  import { t } from "../../lib/i18n/index.svelte";
  import TextField from "../../lib/ui/TextField.svelte";

  let { card = $bindable() }: { card: CardEdit } = $props();
</script>

<div class="form">
  <TextField label={t("Cardholder name")} bind:value={card.holder} autocomplete="off" />
  <TextField label={t("Number")} bind:value={card.number} type="password" inputmode="numeric" autocomplete="off" />
  <TextField label={t("Expiry")} bind:value={card.expiry} autocomplete="off" />
  <TextField label={t("Security code")} bind:value={card.cvv} type="password" inputmode="numeric" autocomplete="off" />
</div>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
</style>
