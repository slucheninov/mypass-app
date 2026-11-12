<script lang="ts">
  // The detail pane of one entry. The owner mounts it keyed by the selected id, so everything
  // local (revealed values, Large Type, the websites sheet) is born and dies with the selection.
  // A selected id that is not in the list (a search narrowed it away) is shown all the same; an id
  // the backend does not know shows the "Item not found" state.
  import { onDestroy, onMount } from "svelte";
  import { api, ApiFailure } from "../lib/api";
  import { events, type EntryDetailDto_Serialize, type FieldRef } from "../lib/bindings";
  import { formatDate } from "../lib/i18n/format";
  import { t } from "../lib/i18n/index.svelte";
  import { lockGuard } from "../lib/lock-guard";
  import { createRevealGroup } from "../lib/secret.svelte";
  import { entries } from "../lib/stores/entries.svelte";
  import { navigation } from "../lib/stores/navigation.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import Avatar from "../lib/ui/Avatar.svelte";
  import Button from "../lib/ui/Button.svelte";
  import FieldRow from "../lib/ui/FieldRow.svelte";
  import IconButton from "../lib/ui/IconButton.svelte";
  import Modal from "../lib/ui/Modal.svelte";
  import AddressDetail from "./AddressDetail.svelte";
  import CardDetail from "./CardDetail.svelte";
  import CustomField from "./CustomField.svelte";
  import WebsitesSheet from "./editors/WebsitesSheet.svelte";
  import FilesDetail from "./FilesDetail.svelte";
  import LargeType from "./LargeType.svelte";
  import LoginDetail from "./LoginDetail.svelte";
  import NotesDetail from "./NotesDetail.svelte";

  let {
    id,
    onopenhistory,
  }: {
    id: string;
    /** Opens the history modal (Task 10). Without it the "View history" link is not shown. */
    onopenhistory?: (id: string) => void;
  } = $props();

  // One group per selected entry: every revealed value of this pane excludes the others.
  const group = createRevealGroup();

  let detail = $state<EntryDetailDto_Serialize | null>(null);
  let missing = $state(false);
  let largeType = $state<FieldRef | null>(null);
  let confirmingPurge = $state(false);
  let generation = 0;

  const guard = (work: () => Promise<unknown>) => toasts.guard(work);
  const binEnabled = $derived(entries.sidebar?.recycleBinEnabled ?? true);

  async function load() {
    const mine = ++generation;
    const unchanged = lockGuard();
    try {
      const next = await api.entryGet(id);
      if (mine !== generation || !unchanged()) return;
      // Content changed (an edit, a sync): a revealed value could be stale, so it goes.
      if (detail && detail.modified !== next.modified) group.hideAll();
      detail = next;
      missing = false;
    } catch (e) {
      if (mine !== generation || !unchanged()) return;
      if (e instanceof ApiFailure && e.code === "notFound") {
        detail = null;
        missing = true;
      } else toasts.reportError(e);
    }
  }

  // `unlisten` answers with a promise that can reject while the window is torn down; that is not
  // actionable, so it is swallowed.
  const quietly = (off: (() => unknown) | null) => {
    try {
      void Promise.resolve(off?.()).catch(() => undefined);
    } catch {
      // Nothing left to unhook.
    }
  };

  onMount(() => {
    void load();
    let off: (() => unknown) | null = null;
    let gone = false;
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
    group.destroy();
    // The websites sheet's model is ours; an editor's model (a different presentation) is not.
    if (navigation.presentation === "browse" && navigation.editModel?.id === id) navigation.editModel = null;
  });

  function showLargeType(field: FieldRef) {
    group.hideAll();
    largeType = field;
  }

  // ---- header actions ----

  const toggleFavorite = () => guard(() => api.entrySetFavorite(id, !(detail?.favorite ?? false)));
  const edit = () =>
    guard(async () => {
      const unchanged = lockGuard();
      const model = await api.entryEditModel(id);
      if (unchanged()) navigation.openEditor(model);
    });
  const restore = () => guard(() => api.entryRestore(id, null));

  function showGroup() {
    if (!detail) return;
    // `id` is a live view of the selection, which `setScope` clears: read it first.
    const mine = id;
    // Selecting a scope drops the selection; the entry stays selected so its pane stays open.
    navigation.setScope({ kind: "group", groupId: detail.groupId });
    navigation.select(mine);
  }

  /** Where the selection goes when this entry leaves the list: the row below, else the one above. */
  function neighbour(): string | null {
    const items = entries.items;
    const at = items.findIndex((e) => e.id === id);
    return items[at + 1]?.id ?? items[at - 1]?.id ?? null;
  }

  const remove = (permanent: boolean) =>
    guard(async () => {
      const next = neighbour();
      const unchanged = lockGuard();
      if (permanent) await api.entryPurge(id);
      else await api.entryDelete(id);
      if (unchanged() && navigation.selectedId === id) navigation.select(next);
    });

  function requestDelete() {
    if (detail?.inRecycleBin || !binEnabled) confirmingPurge = true;
    else void remove(false);
  }

  // ---- change the websites in place ----

  const changeWebsite = () =>
    guard(async () => {
      const unchanged = lockGuard();
      const model = await api.entryEditModel(id);
      // The model is held by the store (nulled on cancel, save, lock and selection change) while
      // the presentation stays "browse": this is not the full editor.
      if (unchanged() && navigation.presentation === "browse") navigation.editModel = model;
    });

  async function saveWebsites() {
    const model = navigation.editModel;
    if (!model) return;
    try {
      await api.entrySave($state.snapshot(model));
      navigation.editModel = null;
      void load();
    } catch (e) {
      toasts.reportError(e);
    }
  }

  const sheetModel = $derived(
    navigation.presentation === "browse" && navigation.editModel?.id === id ? navigation.editModel : null,
  );
</script>

{#if detail}
  <article class="detail" aria-labelledby="entry-title-{id}">
    <header>
      <Avatar text={detail.title} size={44} />
      <div class="titles">
        <h2 id="entry-title-{id}">{detail.title}</h2>
        <button type="button" class="link" onclick={showGroup}>{detail.groupName}</button>
      </div>
      <div class="actions">
        {#if detail.inRecycleBin}
          <Button onclick={restore}>{t("Restore")}</Button>
          <Button variant="danger" onclick={requestDelete}>{t("Delete permanently")}</Button>
        {:else}
          <IconButton
            icon="star"
            label={detail.favorite ? t("Remove from Favorites") : t("Add to Favorites")}
            onclick={toggleFavorite}
          />
          <Button onclick={edit}>{t("Edit")}</Button>
          <Button variant="danger" onclick={requestDelete}>{binEnabled ? t("Delete") : t("Delete permanently")}</Button>
        {/if}
      </div>
    </header>

    <div class="rows">
      {#if detail.entryType === "login"}
        <LoginDetail {detail} {group} onlargetype={showLargeType} onchangewebsite={changeWebsite} />
      {:else if detail.entryType === "address" && detail.address}
        <AddressDetail address={detail.address} />
      {:else if detail.entryType === "card" && detail.card}
        <CardDetail entryId={id} card={detail.card} {group} onlargetype={showLargeType} />
      {/if}
      {#each detail.customFields as field (field.key)}
        <CustomField entryId={id} {field} {group} onlargetype={showLargeType} />
      {/each}
      <FieldRow label={t("Modified")}>{detail.modified === null ? "" : formatDate(detail.modified, "datetime")}</FieldRow>
    </div>

    {#if detail.hasNotes}<NotesDetail entryId={id} {group} />{/if}
    {#if detail.attachments.length > 0}<FilesDetail entryId={id} attachments={detail.attachments} />{/if}
    {#if onopenhistory && detail.historyCount > 0}
      <button type="button" class="link history" onclick={() => onopenhistory(id)}>
        {t("View history (%lld)", detail.historyCount)}
      </button>
    {/if}
  </article>
{:else if missing}
  <div class="missing" role="status">{t("Item not found")}</div>
{/if}

<!-- Siblings of the rows, so a row re-render cannot leak or recreate them. -->
{#if largeType}
  <LargeType entryId={id} field={largeType} onclose={() => (largeType = null)} />
{/if}

{#if sheetModel}
  <WebsitesSheet model={sheetModel} onsave={saveWebsites} oncancel={() => (navigation.editModel = null)} />
{/if}

{#if confirmingPurge}
  <Modal open={true} title={t("Delete permanently")} onclose={() => (confirmingPurge = false)}>
    <p>{t("This entry will be permanently deleted. This cannot be undone.")}</p>
    {#snippet footer()}
      <Button onclick={() => (confirmingPurge = false)}>{t("Cancel")}</Button>
      <Button
        variant="danger"
        onclick={() => {
          confirmingPurge = false;
          void remove(true);
        }}>{t("Delete permanently")}</Button
      >
    {/snippet}
  </Modal>
{/if}

<style>
  .detail {
    padding: 20px 24px;
  }
  header {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 12px;
  }
  .titles {
    flex: 1;
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .history {
    display: block;
    margin-top: 16px;
  }
  .missing {
    padding: 40px 24px;
    text-align: center;
    color: var(--text-2);
  }
</style>
