<script lang="ts">
  // The editor of one entry. The owner (MainApp) mounts one instance per `navigation.editModel`
  // and binds it, so this component edits the store-held model in place; the model carries the
  // password and is nulled by `navigation.closeEditor` on save, cancel and (through the store) lock
  // and vault switch. Dirtiness is a comparison with a snapshot taken when the editor opened; the
  // snapshot is dropped with the component. Errors follow R15: `invalidInput` is one inline
  // message, `limitExceeded` goes to the Files form, everything else is a toast.
  //
  // The editor of an existing entry follows the vault underneath it (Empty Deleted, deleting a
  // parent group, a sync). One rule, in two halves:
  //   1. The entry is gone (`entryGet` answers `notFound`, e.g. Empty Deleted purged it): there is
  //      nothing left to save into, so the editor closes at once with a notice and the draft is
  //      dropped (the backend has already dropped its note drafts).
  //   2. The entry still exists but what it would be saved into is removed: the group chosen in the
  //      editor was deleted or purged, or the entry itself sits in the recycle bin. The editor stays
  //      open and the draft is kept, an inline alert says why, and Save is refused (nothing is sent)
  //      until the cause is gone: the user picks another live group (saving then moves the entry
  //      there, out of the bin too) or cancels. The backend refuses a save into a removed group too
  //      (`save_entry`).
  import { events } from "../lib/bindings";
  import { onDestroy, onMount, tick, untrack } from "svelte";
  import { api, ApiFailure, errorMessage } from "../lib/api";
  import type { AddressDto, EntryEditModel_Serialize, EntryTypeDto } from "../lib/bindings";
  import { findGroup, isLiveGroup } from "../lib/group-tree";
  import { t } from "../lib/i18n/index.svelte";
  import { lockGuard } from "../lib/lock-guard";
  import { entries } from "../lib/stores/entries.svelte";
  import { navigation } from "../lib/stores/navigation.svelte";
  import { session } from "../lib/stores/session.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import { quietly } from "../lib/unlisten";
  import Button from "../lib/ui/Button.svelte";
  import FieldRow from "../lib/ui/FieldRow.svelte";
  import Modal from "../lib/ui/Modal.svelte";
  import TextField from "../lib/ui/TextField.svelte";
  import Toggle from "../lib/ui/Toggle.svelte";
  import AddressForm from "./editors/AddressForm.svelte";
  import CardForm from "./editors/CardForm.svelte";
  import FilesForm from "./editors/FilesForm.svelte";
  import Generator from "./Generator.svelte";
  import LoginForm from "./editors/LoginForm.svelte";
  import NotesForm from "./editors/NotesForm.svelte";
  import GroupPicker from "./GroupPicker.svelte";

  let { model = $bindable() }: { model: EntryEditModel_Serialize } = $props();

  const blankAddress = (): AddressDto => ({
    label: "Home",
    fullName: "",
    givenName: "",
    familyName: "",
    organization: "",
    addressLine1: "",
    addressLine2: "",
    city: "",
    region: "",
    postalCode: "",
    countryCode: "",
    countryName: "",
    phone: "",
    email: "",
  });

  // The backend sends the address and card of those types; a model without them still gets a form.
  // This runs before the baseline is taken, so it never counts as a change.
  untrack(() => {
    if (model.entryType === "address" && !model.address) model.address = blankAddress();
    if (model.entryType === "card" && !model.card) model.card = { holder: "", number: "", expiry: "", cvv: "" };
  });

  // Dropped with the component (it holds the password like the model does).
  let baseline: string | null = untrack(() => JSON.stringify($state.snapshot(model)));
  const changed = () => baseline !== null && JSON.stringify($state.snapshot(model)) !== baseline;
  // A proposal from the browser counts as unsaved even untouched: it was taken from the backend
  // and cannot be fetched again, so leaving it (another entry, a lock by the user, quit) asks first.
  const isDirty = () => changed() || navigation.editorSource === "browser";

  let saving = $state(false);
  let confirming = $state(false);
  let picking = $state(false);
  let titleError = $state<string | null>(null);
  let formError = $state<string | null>(null);
  let filesError = $state<string | null>(null);
  let titleInput: HTMLInputElement | undefined = $state();
  let formEl: HTMLFormElement | undefined = $state();
  let notes: { settle(): Promise<void> } | undefined = $state();

  const headings: Record<EntryTypeDto, () => string> = {
    login: () => t("New Login"),
    address: () => t("New Address"),
    card: () => t("New Card"),
    notes: () => t("New Note"),
    files: () => t("New Files item"),
  };
  const heading = $derived(model.id === null ? headings[model.entryType]() : t("Edit"));

  // Unknown until the sidebar has loaded; then a group that is not live was removed under the editor.
  const groupRemoved = $derived(
    entries.sidebar !== null && !isLiveGroup(entries.sidebar.groups, entries.sidebar.rootGroupId, model.groupId),
  );
  const groupName = $derived(
    model.groupId === entries.sidebar?.rootGroupId
      ? t("Vault")
      : groupRemoved
        ? t("Deleted")
        : (findGroup(entries.sidebar?.groups ?? [], model.groupId)?.name ?? ""),
  );
  /** The entry being edited is in the recycle bin (deleted under the editor). */
  let entryInBin = $state(false);
  // Choosing another group is the way out: saving there moves the entry (out of the bin too).
  const groupAtOpen = untrack(() => model.groupId);
  const blocked = $derived(
    groupRemoved
      ? t("The group of this item was deleted. Choose another group to save it.")
      : entryInBin && model.groupId === groupAtOpen
        ? t("This item is in Deleted. Restore it before saving your changes.")
        : null,
  );

  /** Asks the backend whether the entry is still there and where it is (rule above, both halves). */
  async function follow() {
    const id = model.id;
    if (id === null || saving) return;
    const unchanged = lockGuard();
    try {
      const detail = await api.entryGet(id);
      if (!unchanged() || model.id !== id) return;
      entryInBin = detail.inRecycleBin;
    } catch (e) {
      if (!unchanged() || model.id !== id) return;
      if (e instanceof ApiFailure && e.code === "notFound") {
        toasts.push("info", t("This item was deleted, so its editor was closed."));
        close();
      }
      // Any other failure keeps the editor as it is: the next change asks again.
    }
  }

  onMount(() => {
    if (model.id === null) titleInput?.focus();
    navigation.editorDirty = isDirty;
    let off: (() => unknown) | null = null;
    let gone = false;
    if (model.id !== null) {
      void events.vaultChanged
        .listen(() => void follow())
        .then((unlisten) => (gone ? quietly(unlisten) : (off = unlisten)));
    }
    return () => {
      gone = true;
      if (off) quietly(off);
    };
  });
  // The backend's quit flow asks before it discards unsaved changes: report every change of the
  // dirty state (the snapshot inside `isDirty` tracks the whole model).
  $effect(() => navigation.reportEditorDirty(isDirty()));
  onDestroy(() => {
    navigation.reportEditorDirty(false);
    baseline = null;
    if (navigation.editorDirty === isDirty) navigation.editorDirty = null;
  });

  // Another editor was requested over this dirty one: ask before it replaces this model.
  $effect(() => {
    if (navigation.pendingModel || navigation.leavePending) confirming = true;
  });

  function keepEditing() {
    confirming = false;
    navigation.pendingModel = null;
    navigation.answerLeave(false);
  }

  function discard() {
    confirming = false;
    const next = navigation.pendingModel;
    if (navigation.leavePending) navigation.answerLeave(true);
    else if (next) navigation.openEditor(next);
    else close();
  }

  /** Moves focus to the inline error so a keyboard user lands on it. */
  async function focusError() {
    await tick();
    formEl?.querySelector<HTMLElement>('[role="alert"]')?.focus();
  }

  function close() {
    navigation.closeEditor();
  }

  function cancel() {
    if (saving) return;
    if (isDirty()) confirming = true;
    else close();
  }

  /** The banner's Dismiss: discards the proposal at once, unless the user edited it meanwhile. */
  function dismiss() {
    if (saving) return;
    if (changed()) confirming = true;
    else close();
  }

  async function save() {
    if (saving) return;
    titleError = formError = filesError = null;
    if (blocked) {
      // The alert is already on screen (it follows `blocked`); Save only moves focus to it.
      void focusError();
      return;
    }
    const payload = $state.snapshot(model);
    payload.title = payload.title.trim();
    payload.urls = payload.urls.map((u) => u.trim()).filter((u) => u !== "");
    payload.customFields = payload.customFields.filter((f) => f.key.trim() !== "" || f.value !== "");
    // Only a login needs a title here: an address takes its label from the backend when it has none.
    if (payload.entryType === "login" && payload.title === "") {
      titleError = t("Enter a title");
      titleInput?.focus();
      return;
    }
    saving = true;
    const unchanged = lockGuard();
    try {
      const id = await api.entrySave(payload);
      if (!unchanged()) return;
      // A draft write still in flight must land before the clear below, never after it.
      await notes?.settle();
      if (!unchanged()) return;
      const previous = payload.id;
      navigation.select(id);
      close();
      // The saved note supersedes its drafts (R30); a failure here is invisible and harmless.
      if (previous !== null) void api.draftClear(previous).catch(() => undefined);
    } catch (e) {
      if (!unchanged()) return;
      if (e instanceof ApiFailure && e.code === "limitExceeded") {
        filesError = errorMessage(e);
        void focusError();
      } else if (e instanceof ApiFailure && e.code === "invalidInput") {
        formError = errorMessage(e);
        void focusError();
      } else toasts.reportError(e);
    } finally {
      saving = false;
    }
  }

  // Mod+Enter saves, Escape cancels. A dialog of the editor (websites, group picker, discard) owns
  // its own keys: the event is ignored when it comes from inside one.
  function onkeydown(event: KeyboardEvent) {
    if (event.defaultPrevented) return;
    if (event.target instanceof Element && event.target.closest('[role="dialog"]')) return;
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      void save();
    } else if (event.key === "Escape") {
      event.preventDefault();
      cancel();
    }
  }

  // A lock drops the model through the store; nothing here may outlive it.
  const epoch = session.lockEpoch;
  $effect(() => {
    if (session.lockEpoch !== epoch) baseline = null;
  });
</script>

<svelte:window {onkeydown} />

<form bind:this={formEl} class="editor" onsubmit={(e) => e.preventDefault()} aria-labelledby="editor-title">
  <header>
    <h2 id="editor-title">{heading}</h2>
    <div class="actions">
      <Button disabled={saving} onclick={cancel}>{t("Cancel")}</Button>
      <Button variant="primary" disabled={saving} onclick={() => void save()}>{t("Save")}</Button>
    </div>
  </header>

  {#if navigation.editorSource === "browser"}
    <!-- A save proposal from the browser: nothing is stored until Save; Dismiss is the same as Cancel. -->
    <div class="banner" data-testid="proposal-banner">
      <span>{t("Saving from your browser")}</span>
      <Button variant="plain" disabled={saving} onclick={dismiss}>{t("Dismiss")}</Button>
    </div>
  {/if}

  {#if blocked}<p class="form-error" role="alert" tabindex="-1">{blocked}</p>{/if}
  {#if formError}<p class="form-error" role="alert" tabindex="-1">{formError}</p>{/if}

  <div class="body">
    <TextField label={t("Title")} bind:value={model.title} bind:input={titleInput} error={titleError} />

    {#if model.entryType === "login"}
      <LoginForm bind:model>
        {#snippet generatorSlot({ onuse, host, recordUrl })}
          <Generator {host} {recordUrl} {onuse} />
        {/snippet}
      </LoginForm>
    {:else if model.entryType === "address" && model.address}
      <AddressForm bind:address={model.address} />
    {:else if model.entryType === "card" && model.card}
      <CardForm bind:card={model.card} />
    {:else if model.entryType === "notes"}
      <NotesForm bind:this={notes} bind:model />
    {:else if model.entryType === "files"}
      <FilesForm bind:model error={filesError} />
    {/if}

    <FieldRow label={t("Group")}>
      <Button variant="plain" onclick={() => (picking = true)}>{groupName}</Button>
    </FieldRow>
    <FieldRow label={t("Favorite")}>
      <Toggle checked={model.favorite} label={t("Favorite")} onchange={(v) => (model.favorite = v)} />
    </FieldRow>
  </div>
</form>

<GroupPicker
  open={picking}
  title={t("Group")}
  current={model.groupId}
  onpick={(id) => {
    model.groupId = id;
    picking = false;
  }}
  onclose={() => (picking = false)}
/>

{#if confirming}
  <Modal open={true} title={t("Discard changes?")} onclose={keepEditing}>
    <p>{t("Your changes to this item will be lost.")}</p>
    {#snippet footer()}
      <Button onclick={keepEditing}>{t("Keep editing")}</Button>
      <Button variant="danger" onclick={discard}>{t("Discard")}</Button>
    {/snippet}
  </Modal>
{/if}

<style>
  .editor {
    padding: 20px 24px;
  }
  header {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 14px;
  }
  h2 {
    flex: 1;
    margin: 0;
    font-size: 18px;
    font-weight: 600;
  }
  .actions {
    display: flex;
    gap: 6px;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin: 0 0 12px;
    padding: 6px 10px;
    border: var(--sep);
    border-radius: var(--radius);
    background: var(--list-bg);
    color: var(--text-2);
    font-size: 13px;
  }
  .form-error {
    margin: 0 0 12px;
    padding: 8px 10px;
    border: 1px solid var(--danger);
    border-radius: var(--radius);
    color: var(--danger);
  }
</style>
