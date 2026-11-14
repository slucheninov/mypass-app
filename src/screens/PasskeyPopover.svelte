<script lang="ts">
  // The details of an entry's passkey, fetched only when the user opens it (Show). The credential id
  // is public but is never held before that click; it lives in this component only, and a lock or
  // closing drops it. The private key and the user handle never reach the webview at all.
  import { onDestroy, onMount } from "svelte";
  import { api } from "../lib/api";
  import type { PasskeyDetails_Serialize } from "../lib/bindings";
  import { formatDate } from "../lib/i18n/format";
  import { t } from "../lib/i18n/index.svelte";
  import { lockGuard } from "../lib/lock-guard";
  import { session } from "../lib/stores/session.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import FieldRow from "../lib/ui/FieldRow.svelte";

  let {
    entryId,
    context,
    onclose,
  }: {
    entryId: string;
    /** The footnote about removing the key belongs to the read-only detail only. */
    context: "detail" | "editor";
    onclose: () => void;
  } = $props();

  let details = $state<PasskeyDetails_Serialize | null>(null);

  onMount(() => {
    const unchanged = lockGuard();
    api
      .entryPasskeyDetails(entryId)
      .then((found) => {
        if (!unchanged()) return;
        // A damaged or vanished passkey has no details to show.
        if (found === null) onclose();
        else details = found;
      })
      .catch((e: unknown) => {
        if (!unchanged()) return;
        toasts.reportError(e);
        onclose();
      });
  });
  onDestroy(() => (details = null));

  // A lock drops the details and closes the popover.
  const epoch = session.lockEpoch;
  $effect(() => {
    if (session.lockEpoch !== epoch) {
      details = null;
      onclose();
    }
  });
</script>

{#if details}
  <div class="popover" role="region" aria-label={t("Passkey")}>
    <FieldRow label={t("Site")}>{details.relyingParty}</FieldRow>
    {#if details.username !== ""}
      <FieldRow label={t("Username")}>{details.username}</FieldRow>
    {/if}
    <FieldRow label={t("Created")}>
      {details.created !== null ? formatDate(details.created) : t("Unknown — created outside MyPass")}
    </FieldRow>
    <FieldRow label={t("Credential ID")}><span class="mono">{details.credentialId}</span></FieldRow>
    {#if context === "detail"}
      <p class="note">{t("Remove this passkey by editing the entry.")}</p>
    {/if}
  </div>
{/if}

<style>
  .popover {
    margin-top: 6px;
    padding: 4px 12px;
    border: var(--sep);
    border-radius: var(--radius);
    background: var(--bg);
    box-shadow: var(--shadow);
  }
  .mono {
    font-family: var(--mono, ui-monospace, SFMono-Regular, Menlo, monospace);
    font-size: 12px;
  }
  .note {
    margin: 6px 0 4px;
    color: var(--text-2);
    font-size: 12px;
  }
</style>
