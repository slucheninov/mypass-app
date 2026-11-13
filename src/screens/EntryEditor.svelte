<script lang="ts">
  // The editor of one entry. The owner (MainApp) mounts one instance per `navigation.editModel`
  // and binds it, so this component edits the store-held model in place; the model carries the
  // password and is nulled by `navigation.closeEditor` on save, cancel and (through the store) lock
  // and vault switch. Dirtiness is a comparison with a snapshot taken when the editor opened; the
  // snapshot is dropped with the component. Errors follow R15: `invalidInput` is one inline
  // message, `limitExceeded` goes to the Files form, everything else is a toast.
  import { onDestroy, onMount, tick, untrack } from "svelte";
  import { api, ApiFailure, errorMessage } from "../lib/api";
  import type { AddressDto, EntryEditModel_Serialize, EntryTypeDto } from "../lib/bindings";
  import { findGroup } from "../lib/group-tree";
  import { t } from "../lib/i18n/index.svelte";
  import { lockGuard } from "../lib/lock-guard";
  import { entries } from "../lib/stores/entries.svelte";
  import { navigation } from "../lib/stores/navigation.svelte";
  import { session } from "../lib/stores/session.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import Button from "../lib/ui/Button.svelte";
  import FieldRow from "../lib/ui/FieldRow.svelte";
  import Modal from "../lib/ui/Modal.svelte";
  import TextField from "../lib/ui/TextField.svelte";
  import Toggle from "../lib/ui/Toggle.svelte";
  import AddressForm from "./editors/AddressForm.svelte";
  import CardForm from "./editors/CardForm.svelte";
  import FilesForm from "./editors/FilesForm.svelte";
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
  const isDirty = () => baseline !== null && JSON.stringify($state.snapshot(model)) !== baseline;

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

  const groupName = $derived(
    model.groupId === entries.sidebar?.rootGroupId
      ? t("Vault")
      : (findGroup(entries.sidebar?.groups ?? [], model.groupId)?.name ?? ""),
  );

  onMount(() => {
    if (model.id === null) titleInput?.focus();
    navigation.editorDirty = isDirty;
  });
  onDestroy(() => {
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

  async function save() {
    if (saving) return;
    titleError = formError = filesError = null;
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

  {#if formError}<p class="form-error" role="alert" tabindex="-1">{formError}</p>{/if}

  <div class="body">
    <TextField label={t("Title")} bind:value={model.title} bind:input={titleInput} error={titleError} />

    {#if model.entryType === "login"}
      <LoginForm bind:model />
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
  .form-error {
    margin: 0 0 12px;
    padding: 8px 10px;
    border: 1px solid var(--danger);
    border-radius: var(--radius);
    color: var(--danger);
  }
</style>
