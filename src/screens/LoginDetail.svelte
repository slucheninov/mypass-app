<script lang="ts">
  // The field rows of a login: username, password, one-time code, websites, the passkey chip.
  // Secrets are members of the entry's reveal group; the websites are links only for http(s).
  import { onDestroy, untrack } from "svelte";
  import type { EntryDetailDto_Serialize, FieldRef } from "../lib/bindings";
  import { formatDate } from "../lib/i18n/format";
  import { t } from "../lib/i18n/index.svelte";
  import type { RevealGroup } from "../lib/secret.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import Button from "../lib/ui/Button.svelte";
  import FieldRow from "../lib/ui/FieldRow.svelte";
  import Icon from "../lib/ui/Icon.svelte";
  import ProviderMark from "../lib/ui/ProviderMark.svelte";
  import { PROVIDER_NAME } from "../lib/sign-in";
  import { webLinks } from "../lib/ui/web-links";
  import { webUrl } from "../lib/web-url";
  import CopyFieldValue from "./CopyFieldValue.svelte";
  import DetailField from "./DetailField.svelte";
  import PasskeyPopover from "./PasskeyPopover.svelte";
  import TotpField from "./TotpField.svelte";
  import { copyField } from "./detail-actions";

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

  function copyUrl(index: number) {
    return toasts.guard(() => copyField(entryId, { kind: "urlAt", index }));
  }
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
    {#snippet valueWithActions()}
      <ul class="urls" use:webLinks>
        {#each detail.urls as raw, i (i)}
          {@const url = webUrl(raw)}
          <li>
            <CopyFieldValue onclick={() => copyUrl(i)}>{raw}</CopyFieldValue>
            {#if url !== null}
              <a href={url} aria-label={`${t("Open website")} ${raw}`} title={t("Open website")}>
                <Icon name="link" size={14} />
              </a>
            {/if}
          </li>
        {/each}
      </ul>
    {/snippet}
    {#snippet extra()}
      <Button variant="plain" onclick={onchangewebsite}>{t("Change…")}</Button>
    {/snippet}
  </DetailField>
{/if}
{#if detail.signIn}
  {@const signIn = detail.signIn}
  <div role="group" aria-label={t("Sign-in method")} data-testid="sign-in-method">
    <FieldRow label={t("Sign-in method")}>
      <span class="added"
        ><ProviderMark provider={signIn.provider} />{PROVIDER_NAME[signIn.provider]}{#if signIn.lastUsed != null}<span class="muted"
            >{t("Last used %@", [formatDate(signIn.lastUsed)])}</span
          >{/if}</span
      >
    </FieldRow>
  </div>
{/if}
{#if detail.passkey}
  {@const passkey = detail.passkey}
  <div class="passkey" role="group" aria-label={t("Passkey")}>
    <FieldRow label={t("Passkey")}>
      <span class="added"><Icon name="passkey" size={14} />{#if !passkey.usable}{t("Damaged passkey")}{:else if passkey.created !== null}{t("Added %@", [formatDate(passkey.created)])}{:else}{t("Added")}{/if}</span>
      {#snippet actions()}
        {#if passkey.usable}
          <Button variant="plain" aria-label={t("Show passkey details")} aria-expanded={passkeyOpen} onclick={() => (passkeyOpen = !passkeyOpen)}>
            {t("Show")}
          </Button>
        {/if}
      {/snippet}
    </FieldRow>
    {#if passkeyOpen && passkey.usable}
      <PasskeyPopover {entryId} context="detail" onclose={() => (passkeyOpen = false)} />
    {:else if !passkey.usable}
      <p class="note">{t("Remove this passkey by editing the entry.")}</p>
    {/if}
  </div>
{/if}

<style>
  .urls {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .urls li {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .urls a {
    display: inline-flex;
    flex: none;
    padding: 4px;
    border-radius: var(--radius);
    color: var(--accent);
  }
  .urls a:hover {
    background: var(--hover);
  }
  .added {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .muted {
    color: var(--text-2);
    font-size: 12px;
  }
  .note {
    margin: 6px 0 4px;
    color: var(--text-2);
    font-size: 12px;
  }
</style>
