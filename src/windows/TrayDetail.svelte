<script lang="ts">
  // The tray panel's compact entry view (ported from the old menu-bar panel): username, password
  // (masked, Reveal / Copy), one-time code, website, modified. Nothing secret is in the DOM before
  // an explicit Reveal; the one revealed value is dropped on Back, on window hide and on lock.
  import { onMount } from "svelte";
  import { api, errorMessage } from "../lib/api";
  import { events, type EntryDetailDto_Serialize } from "../lib/bindings";
  import { quietly } from "../lib/unlisten";
  import { formatDate } from "../lib/i18n/format";
  import { t } from "../lib/i18n/index.svelte";
  import { lockGuard } from "../lib/lock-guard";
  import { createRevealed } from "../lib/secret.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import Button from "../lib/ui/Button.svelte";
  import TrayRow from "./TrayRow.svelte";
  import TrayTotp from "./TrayTotp.svelte";

  let { id, onback }: { id: string; onback: () => void } = $props();

  let detail = $state<EntryDetailDto_Serialize | null>(null);
  let failure = $state<string | null>(null);
  const password = createRevealed();
  // The first stored website: the one `entry_copy` of a `url` field copies.
  const website = $derived(detail?.url ?? detail?.urls[0] ?? "");

  let requests = 0;

  /** Loads (or reloads) the entry; a changed `modified` hides a revealed value, a vanished entry
   *  replaces the fields with its failure text. */
  async function load(): Promise<void> {
    const mine = ++requests;
    const unchanged = lockGuard();
    try {
      const loaded = await api.entryGet(id);
      if (mine !== requests || !unchanged()) return;
      if (detail && detail.modified !== loaded.modified) password.hide();
      detail = loaded;
      failure = null;
    } catch (e) {
      if (mine !== requests || !unchanged()) return;
      password.hide();
      detail = null;
      failure = errorMessage(e);
    }
  }

  onMount(() => {
    void load();
    let off: (() => void) | null = null;
    let gone = false;
    void events.vaultChanged
      .listen(() => void load())
      .then((unlisten) => {
        if (gone) quietly(unlisten);
        else off = unlisten;
      })
      .catch(() => {});
    return () => {
      gone = true;
      requests++;
      if (off) quietly(off);
      password.hide();
    };
  });

  const edit = () => toasts.guard(() => api.appOpenEntry(id, true));
</script>

<section class="detail" aria-label={detail?.title ?? t("Recent items")}>
  <header>
    <Button variant="plain" onclick={onback}>{t("Back")}</Button>
    {#if detail}<Button onclick={() => void edit()}>{t("Edit")}</Button>{/if}
  </header>
  {#if failure}
    <p class="failure" role="status">{failure}</p>
  {:else if detail}
    <h2>{detail.title}</h2>
    {#if detail.username !== ""}
      <TrayRow label={t("Username")} entryId={id} field={{ kind: "username" }} text={detail.username} />
    {/if}
    {#if detail.hasPassword}
      <TrayRow label={t("Password")} entryId={id} field={{ kind: "password" }} secret={password} />
    {/if}
    {#if detail.totpPeriod !== null}<TrayTotp entryId={id} />{/if}
    {#if website !== ""}
      <!-- Plain selectable text: opening links is the main window's job (R10). -->
      <TrayRow label={t("Website")} entryId={id} field={{ kind: "url" }} text={website} />
    {/if}
    {#if detail.modified !== null}
      <TrayRow label={t("Modified")} entryId={id} text={formatDate(detail.modified)} />
    {/if}
  {/if}
</section>

<style>
  .detail {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
  }
  header {
    display: flex;
    justify-content: space-between;
    padding-bottom: 6px;
  }
  h2 {
    margin: 0 0 4px;
    font-size: 15px;
    overflow-wrap: anywhere;
  }
  .failure {
    color: var(--text-2);
  }
</style>
