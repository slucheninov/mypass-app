<script lang="ts">
  // The field rows of a login: username, password, one-time code, websites, the passkey chip.
  // Secrets are members of the entry's reveal group; the websites are links only for http(s).
  import { onDestroy, untrack } from "svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import type { EntryDetailDto_Serialize, FieldRef } from "../lib/bindings";
  import { formatDate } from "../lib/i18n/format";
  import { t } from "../lib/i18n/index.svelte";
  import type { RevealGroup } from "../lib/secret.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import Button from "../lib/ui/Button.svelte";
  import FieldRow from "../lib/ui/FieldRow.svelte";
  import Icon from "../lib/ui/Icon.svelte";
  import { webUrl } from "../lib/web-url";
  import DetailField from "./DetailField.svelte";
  import TotpField from "./TotpField.svelte";

  let {
    detail,
    group,
    onlargetype,
    onchangewebsite,
  }: {
    detail: EntryDetailDto_Serialize;
    group: RevealGroup;
    onlargetype: (field: FieldRef) => void;
    onchangewebsite: () => void;
  } = $props();

  const password = untrack(() => group.create());
  onDestroy(() => password.hide());
  const entryId = $derived(detail.id);
  let passkeyOpen = $state(false);

  const open = (event: MouseEvent, url: string) => {
    event.preventDefault();
    void openUrl(url).catch((e) => toasts.reportError(e));
  };
</script>

{#if detail.username !== ""}
  <DetailField label={t("Username")} {entryId} field={{ kind: "username" }} text={detail.username} {onlargetype} />
{/if}
{#if detail.hasPassword}
  <DetailField label={t("Password")} {entryId} field={{ kind: "password" }} secret={password} {onlargetype} />
{/if}
{#if detail.totpPeriod !== null}
  <TotpField {entryId} {onlargetype} />
{/if}
{#if detail.urls.length > 0}
  <DetailField label={t("Website")} {entryId} field={{ kind: "url" }} {onlargetype}>
    <ul class="urls">
      {#each detail.urls as raw, i (i)}
        {@const url = webUrl(raw)}
        <li>
          {#if url !== null}
            <a href={url} onclick={(e) => open(e, url)}>{raw}</a>
          {:else}
            {raw}
          {/if}
        </li>
      {/each}
    </ul>
    {#snippet extra()}
      <Button variant="plain" onclick={onchangewebsite}>{t("Change…")}</Button>
    {/snippet}
  </DetailField>
{/if}
{#if detail.passkey}
  <div class="passkey">
    <button
      type="button"
      class="chip"
      aria-expanded={passkeyOpen}
      onclick={() => (passkeyOpen = !passkeyOpen)}
      onkeydown={(e) => e.key === "Escape" && (passkeyOpen = false)}
    >
      <Icon name="passkey" size={14} />{t("Passkey")}
    </button>
    {#if passkeyOpen}
      <div class="popover" role="region" aria-label={t("Passkey")}>
        <FieldRow label={t("Relying party")}>{detail.passkey.relyingParty}</FieldRow>
        <FieldRow label={t("Username")}>{detail.passkey.username}</FieldRow>
        {#if detail.passkey.created !== null}
          <FieldRow label={t("Created")}>{formatDate(detail.passkey.created)}</FieldRow>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .urls {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  a {
    color: var(--accent);
  }
  .passkey {
    margin-top: 10px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 3px 10px;
    border: 1px solid var(--control-border);
    border-radius: 999px;
    background: var(--control-bg);
    color: var(--text);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .popover {
    margin-top: 6px;
    padding: 4px 12px;
    border: var(--sep);
    border-radius: var(--radius);
    background: var(--bg);
    box-shadow: var(--shadow);
  }
</style>
