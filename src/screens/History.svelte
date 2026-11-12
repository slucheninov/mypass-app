<script lang="ts">
  // The earlier versions of one entry, newest first. Only the date, the title and the NAMES of the
  // fields that changed are shown: an old password or note never reaches the webview (there is no
  // backend command that reveals a history value), so there is nothing to reveal, copy or clear.
  // The owner closes it when the selection, the vault or the lock state changes.
  import { onDestroy, onMount, tick } from "svelte";
  import { api, ApiFailure } from "../lib/api";
  import { events, type HistoryItem } from "../lib/bindings";
  import { formatDate } from "../lib/i18n/format";
  import { t } from "../lib/i18n/index.svelte";
  import { lockGuard } from "../lib/lock-guard";
  import { toasts } from "../lib/stores/toasts.svelte";
  import Button from "../lib/ui/Button.svelte";
  import Modal from "../lib/ui/Modal.svelte";

  let { entryId, open, onclose }: { entryId: string; open: boolean; onclose: () => void } = $props();

  const uid = $props.id();
  const optionId = (index: number) => `history-${uid}-${index}`;

  let items = $state<HistoryItem[] | null>(null);
  let active = $state(0);
  let confirming = $state<HistoryItem | null>(null);
  let busy = false;
  let listbox: HTMLElement | undefined = $state();
  let generation = 0;

  /** A known KDBX field key as a localised label (R29); an unknown custom key is user data, kept as is. */
  function fieldLabel(key: string): string {
    switch (key) {
      case "Title":
        return t("Title");
      case "UserName":
        return t("Username");
      case "Password":
        return t("Password");
      case "URL":
        return t("Website");
      case "Notes":
      case "MyPass.NotesBody":
        return t("Notes");
      case "otp":
        return t("One-time password");
      case "MyPass.CardNumber":
        return t("Card number");
      case "MyPass.CVV":
        return t("Security code");
      case "MyPass.CardholderName":
        return t("Cardholder");
      case "MyPass.ExpiryDate":
        return t("Expires");
      case "MyPass.URIMatchMode":
        return t("Match mode");
      case "MyPass.EntryType":
        return t("Type");
      case "Attachments":
        return t("Attachments");
      default:
        return key;
    }
  }

  const changed = (item: HistoryItem): string =>
    [...new Set(item.changedFields.map(fieldLabel))].join(", ");
  const when = (item: HistoryItem): string =>
    item.modified === null ? t("Unknown date") : formatDate(item.modified, "datetime");

  /** A version is the same revision across a reload when its date and title match; its `index` is not stable. */
  const sameVersion = (a: HistoryItem, b: HistoryItem) => a.modified === b.modified && a.title === b.title;

  async function load() {
    const mine = ++generation;
    const unchanged = lockGuard();
    try {
      const next = await api.entryHistory(entryId);
      if (mine !== generation || !unchanged()) return;
      // Keep the chosen row and the open confirmation on the same version, not on the same index:
      // a sync or merge may have added or dropped a version meanwhile.
      const selected = items?.[active] ?? null;
      const asked = confirming;
      items = next;
      const at = selected ? next.findIndex((v) => sameVersion(v, selected)) : -1;
      active = at >= 0 ? at : 0;
      confirming = asked ? (next.find((v) => sameVersion(v, asked)) ?? null) : null;
    } catch (e) {
      if (mine !== generation || !unchanged()) return;
      toasts.reportError(e);
      // Nothing to show for an entry the backend no longer knows (or cannot read): close.
      if (items === null || (e instanceof ApiFailure && e.code === "notFound")) onclose();
    }
  }

  onMount(() => {
    void load();
    let off: (() => unknown) | null = null;
    let gone = false;
    const quietly = (fn: (() => unknown) | null) => {
      try {
        void Promise.resolve(fn?.()).catch(() => undefined);
      } catch {
        // Nothing left to unhook.
      }
    };
    void events.vaultChanged
      .listen(() => void load())
      .then((unlisten) => (gone ? quietly(unlisten) : (off = unlisten)));
    return () => {
      gone = true;
      quietly(off);
    };
  });

  onDestroy(() => {
    generation++;
    items = null;
    confirming = null;
  });

  const current = $derived(items?.[active] ?? null);

  function move(to: number) {
    if (!items || items.length === 0) return;
    active = Math.max(0, Math.min(items.length - 1, to));
    void tick().then(() => document.getElementById(optionId(active))?.scrollIntoView?.({ block: "nearest" }));
  }

  function onkeydown(event: KeyboardEvent) {
    switch (event.key) {
      case "ArrowDown":
        move(active + 1);
        break;
      case "ArrowUp":
        move(active - 1);
        break;
      case "Home":
        move(0);
        break;
      case "End":
        move((items?.length ?? 1) - 1);
        break;
      case "Enter":
        if (current) confirming = current;
        break;
      default:
        return;
    }
    event.preventDefault();
  }

  async function restore() {
    const item = confirming;
    confirming = null;
    if (!item || busy) return;
    busy = true;
    const unchanged = lockGuard();
    try {
      await api.entryHistoryRestore(entryId, item.index);
      // The vault-changed event refetches the detail and the list; the modal has done its job.
      if (unchanged()) onclose();
    } catch (e) {
      if (unchanged()) {
        toasts.reportError(e);
        await tick();
        listbox?.focus();
      }
    } finally {
      busy = false;
    }
  }
</script>

<Modal {open} title={t("History")} {onclose}>
  {#if items !== null && items.length === 0}
    <p class="note" role="status">{t("No earlier versions")}</p>
  {:else if items !== null}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div
      class="versions"
      role="listbox"
      tabindex="0"
      aria-label={t("History")}
      aria-activedescendant={optionId(active)}
      bind:this={listbox}
      {onkeydown}
    >
      {#each items as item, position (item.index)}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          class="version"
          role="option"
          id={optionId(position)}
          tabindex="-1"
          aria-selected={position === active}
          onclick={() => (active = position)}
          ondblclick={() => {
            active = position;
            confirming = item;
          }}
        >
          <span class="when">{when(item)}</span>
          <span class="title">{item.title}</span>
          {#if item.changedFields.length > 0}
            <span class="fields">{t("Changed: %@", [changed(item)])}</span>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
  {#snippet footer()}
    <Button onclick={onclose}>{t("Close")}</Button>
    {#if current}
      <Button variant="primary" onclick={() => (confirming = current)}>{t("Restore this version")}</Button>
    {/if}
  {/snippet}
</Modal>

{#if confirming}
  <Modal open={true} title={t("Restore this version")} onclose={() => (confirming = null)}>
    <p>{t("Restore the version from %@? The current version is kept in history.", [when(confirming)])}</p>
    {#snippet footer()}
      <Button onclick={() => (confirming = null)}>{t("Cancel")}</Button>
      <Button variant="primary" onclick={() => void restore()}>{t("Restore")}</Button>
    {/snippet}
  </Modal>
{/if}

<style>
  .versions {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 340px;
    max-height: 340px;
    overflow-y: auto;
    border-radius: var(--radius);
  }
  .version {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 6px 10px;
    border-radius: var(--radius);
    cursor: default;
  }
  .version[aria-selected="true"] {
    background: var(--accent);
    color: var(--accent-text);
  }
  .when {
    font-weight: 600;
  }
  .fields {
    font-size: 12px;
    opacity: 0.85;
  }
  .note {
    margin: 0;
    color: var(--text-2);
  }
</style>
