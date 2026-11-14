<script lang="ts">
  // The login-specific part of the editor: username, password (show/hide, Generate), websites,
  // one-time password, notes and custom fields. Title, group and favourite belong to the editor
  // shell. The Generator popover of a later task plugs in through `generatorSlot`, which receives
  // `onuse` (puts the chosen value into the password field) and the first website for the rules.
  import { onMount, tick, type Snippet } from "svelte";
  import type { EntryEditModel_Serialize } from "../../lib/bindings";
  import { t } from "../../lib/i18n/index.svelte";
  import { onWindowHidden } from "../../lib/secret.svelte";
  import { webUrl } from "../../lib/web-url";
  import Button from "../../lib/ui/Button.svelte";
  import FieldRow from "../../lib/ui/FieldRow.svelte";
  import Modal from "../../lib/ui/Modal.svelte";
  import PasskeyPopover from "../PasskeyPopover.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import TextField from "../../lib/ui/TextField.svelte";
  import CustomFields from "./CustomFields.svelte";
  import WebsitesSheet from "./WebsitesSheet.svelte";

  let {
    model = $bindable(),
    generatorSlot,
  }: {
    model: EntryEditModel_Serialize;
    generatorSlot?: Snippet<[{ onuse: (value: string) => void; host: string; recordUrl: string | null }]>;
  } = $props();

  let shown = $state(false);
  let generating = $state(false);
  let sheetOpen = $state(false);
  let passkeyOpen = $state(false);
  let confirmingRemoval = $state(false);

  // The plain-text password must not stay on screen when the window hides or minimises, like the
  // detail pane's reveals.
  onMount(() => onWindowHidden(() => (shown = false)));

  const firstUrl = $derived(model.urls.map((u) => webUrl(u)).find((u) => u !== null) ?? null);
  const host = $derived.by(() => {
    try {
      return firstUrl ? new URL(firstUrl).hostname : "";
    } catch {
      return "";
    }
  });
  const websiteCount = $derived(model.urls.filter((u) => u.trim() !== "").length);

  function use(value: string) {
    model.password = value;
    generating = false;
  }
</script>

<div class="form">
  <TextField label={t("Username")} bind:value={model.username} autocomplete="off" />
  <div class="password">
    <TextField label={t("Password")} bind:value={model.password} type={shown ? "text" : "password"} autocomplete="off" />
    <IconButton
      icon={shown ? "eye-off" : "eye"}
      label={shown ? t("Hide password") : t("Show password")}
      onclick={() => (shown = !shown)}
    />
    {#if generatorSlot}
      <Button onclick={() => (generating = !generating)}>{t("Generate")}</Button>
    {/if}
  </div>
  {#if generatorSlot && generating}
    {@render generatorSlot({ onuse: use, host, recordUrl: firstUrl })}
  {/if}

  <div class="websites">
    <span class="caption">{t("Websites")}</span>
    <Button onclick={() => (sheetOpen = true)}>{t("%lld websites", websiteCount)}</Button>
  </div>

  <TextField
    label={t("One-time password")}
    bind:value={model.totp}
    type="password"
    placeholder={t("Paste an otpauth link or a secret key")}
    autocomplete="off"
  />

  <label class="notes">
    <span class="caption">{t("Notes")}</span>
    <textarea bind:value={model.notes} rows="4" spellcheck="true"></textarea>
  </label>

  <CustomFields bind:fields={model.customFields} />

  {#if model.passkey}
    <!-- The key is never editable: Remove is deferred to Save (the key is unrecoverable once saved away). -->
    <div role="group" aria-label={t("Passkey")}>
      <FieldRow label={t("Passkey")}>
        {#if model.removePasskey}
          {t("Passkey will be removed when you save")}
        {:else if !model.passkey.usable}
          {t("Damaged passkey")}
        {/if}
        {#snippet actions()}
          {#if model.passkey?.usable && model.id !== null}
            <Button variant="plain" aria-label={t("Show passkey details")} aria-expanded={passkeyOpen} onclick={() => (passkeyOpen = !passkeyOpen)}>
              {t("Show")}
            </Button>
          {/if}
          {#if model.removePasskey}
            <Button variant="plain" id="passkey-undo" onclick={() => (model.removePasskey = false)}>{t("Undo")}</Button>
          {:else}
            <Button variant="plain" onclick={() => (confirmingRemoval = true)}>{t("Remove")}</Button>
          {/if}
        {/snippet}
      </FieldRow>
      {#if passkeyOpen && model.id !== null && model.passkey.usable}
        <PasskeyPopover entryId={model.id} context="editor" onclose={() => (passkeyOpen = false)} />
      {/if}
    </div>
  {/if}
</div>

{#if confirmingRemoval}
  <Modal open={true} title={t("Remove passkey?")} onclose={() => (confirmingRemoval = false)}>
    <p>{t("The passkey is removed when you save. Earlier versions of this item in its history keep the old key.")}</p>
    {#snippet footer()}
      <Button onclick={() => (confirmingRemoval = false)}>{t("Cancel")}</Button>
      <Button
        variant="danger"
        onclick={async () => {
          model.removePasskey = true;
          confirmingRemoval = false;
          // The Remove button that opened this dialog is gone now, so the dialog's focus restore has no
          // target; hand focus to Undo instead of letting it fall to the body.
          await tick();
          document.getElementById("passkey-undo")?.focus();
        }}>{t("Remove")}</Button
      >
    {/snippet}
  </Modal>
{/if}

{#if sheetOpen}
  <WebsitesSheet {model} onsave={() => void (sheetOpen = false)} oncancel={() => void (sheetOpen = false)} />
{/if}

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .password {
    display: flex;
    align-items: flex-end;
    gap: 6px;
  }
  .password :global(.text-field) {
    flex: 1;
  }
  .websites {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .caption {
    color: var(--text-2);
    font-size: 12px;
  }
  .notes {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  textarea {
    padding: 6px 8px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
    background: var(--control-bg);
    color: var(--text);
    font: inherit;
    resize: vertical;
  }
</style>
