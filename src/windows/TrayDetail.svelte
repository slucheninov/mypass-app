<script lang="ts">
  // The tray panel's compact entry view: a Back header, the letter avatar with the title and the
  // last change, then the user name, the password (masked; a click copies it through the backend,
  // Reveal shows it), the one-time code, the passkey and the website. Nothing secret is in the DOM
  // before an explicit Reveal; the one revealed value is dropped on Back, on window hide and on lock.
  import { onMount } from "svelte";
  import { api, errorMessage } from "../lib/api";
  import { events, type EntryDetailDto_Serialize } from "../lib/bindings";
  import { quietly } from "../lib/unlisten";
  import { formatDate } from "../lib/i18n/format";
  import { t } from "../lib/i18n/index.svelte";
  import { lockGuard } from "../lib/lock-guard";
  import { createRevealed } from "../lib/secret.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import Avatar from "../lib/ui/Avatar.svelte";
  import Icon from "../lib/ui/Icon.svelte";
  import IconButton from "../lib/ui/IconButton.svelte";
  import TrayRow from "./TrayRow.svelte";
  import TrayTotp from "./TrayTotp.svelte";

  let { id, onback }: { id: string; onback: () => void } = $props();

  let detail = $state<EntryDetailDto_Serialize | null>(null);
  let failure = $state<string | null>(null);
  const password = createRevealed();
  // The first stored website: the one `entry_copy` of a `url` field copies.
  const website = $derived(detail?.url ?? detail?.urls[0] ?? "");
  // A damaged passkey cannot sign in anywhere; the panel leaves it to the main window.
  const passkey = $derived(detail?.passkey?.usable === false ? null : (detail?.passkey ?? null));

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
  const showInMain = () => toasts.guard(() => api.appOpenEntry(id, false));
</script>

<section class="detail" aria-label={detail?.title ?? t("Recent items")}>
  <header>
    <button type="button" class="back" onclick={onback}>
      <Icon name="chevron-left" size={16} stroke={2} />
      <span>{t("Back")}</span>
    </button>
    {#if detail}
      <div class="tools">
        <IconButton icon="edit" label={t("Edit")} onclick={() => void edit()} />
        <IconButton icon="window" label={t("Open main window")} onclick={() => void showInMain()} />
      </div>
    {/if}
  </header>
  {#if failure}
    <p class="failure" role="status">{failure}</p>
  {:else if detail}
    <div class="card">
      <div class="identity">
        <Avatar text={detail.title} size={52} />
        <div class="names">
          <h2>{detail.title}</h2>
          {#if detail.modified !== null}
            <p class="modified">{t("Last modified: %@", [formatDate(detail.modified)])}</p>
          {/if}
        </div>
      </div>
      <div class="fields">
        {#if detail.username !== ""}
          <TrayRow label={t("Username")} entryId={id} field={{ kind: "username" }} text={detail.username} />
        {/if}
        {#if detail.hasPassword}
          <TrayRow label={t("Password")} entryId={id} field={{ kind: "password" }} secret={password} />
        {/if}
        {#if detail.totpPeriod !== null}<TrayTotp entryId={id} />{/if}
        {#if passkey}
          <TrayRow
            label={t("Passkey")}
            entryId={id}
            text={passkey.created !== null ? t("Created %@", [formatDate(passkey.created)]) : passkey.relyingParty}
          />
        {/if}
        {#if website !== ""}
          <!-- Plain text with copy: opening links is the main window's job (R10). -->
          <TrayRow label={t("Website")} entryId={id} field={{ kind: "url" }} text={website} />
        {/if}
      </div>
    </div>
  {/if}
</section>

<style>
  .detail {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-height: 0;
    overflow-y: auto;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .back {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 4px 8px 4px 2px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--font-size-md);
    font-weight: 600;
    cursor: default;
  }
  .back:hover,
  .back:focus-visible {
    background: var(--hover);
    outline: none;
  }
  .back:focus-visible {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .tools {
    display: flex;
    gap: 2px;
  }
  .card {
    border: var(--sep);
    border-radius: 10px;
    background: var(--list-bg);
  }
  .identity {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 14px 12px;
    border-bottom: var(--sep);
  }
  .names {
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-size: 17px;
    overflow-wrap: anywhere;
  }
  .modified {
    margin: 2px 0 0;
    color: var(--text-2);
    font-size: var(--font-size-sm);
  }
  /* The rows are separate components: the separators between them are drawn here. */
  .fields > :global(.row + .row) {
    border-top: var(--sep);
  }
  .failure {
    color: var(--text-2);
  }
</style>
