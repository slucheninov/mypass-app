<script lang="ts">
  // The login-specific part of the editor: username, password (show/hide, Generate), websites,
  // one-time password, notes and custom fields. Title, group and favourite belong to the editor
  // shell. The Generator popover of a later task plugs in through `generatorSlot`, which receives
  // `onuse` (puts the chosen value into the password field) and the first website for the rules.
  import type { Snippet } from "svelte";
  import type { EntryEditModel_Serialize } from "../../lib/bindings";
  import { t } from "../../lib/i18n/index.svelte";
  import { webUrl } from "../../lib/web-url";
  import Button from "../../lib/ui/Button.svelte";
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
</div>

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
