<script lang="ts">
  import { api, errorMessage } from "../lib/api";
  import type { FolderProbe_Serialize, SecondaryError, SecondaryUnavailable } from "../lib/bindings";
  import { sync } from "../lib/stores/sync.svelte";
  import { session } from "../lib/stores/session.svelte";
  import { settings } from "../lib/stores/settings.svelte";
  import { formatDate } from "../lib/i18n/format";
  import { t } from "../lib/i18n/index.svelte";
  import Button from "../lib/ui/Button.svelte";
  import LocationChangeSheet from "./LocationChangeSheet.svelte";
  let target = $state<FolderProbe_Serialize | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);
  const errors = $derived<Record<SecondaryError, string>>({
    wrongPassword: t("Wrong master password for the vault in that folder"), notAVault: t("That file is not a vault"),
    unsupported: t("Not supported on this system"), corrupt: t("The vault file looks damaged"), differentVault: t("That folder contains a different vault"),
    tooLarge: t("Limit reached"), notRegularFile: t("That file is not a vault"), remoteBusy: t("The vault changed on disk"), io: t("Something went wrong"), watchFailed: t("Folder changes cannot be watched. Try syncing again."),
  });
  const unavailable = $derived<Record<SecondaryUnavailable, string>>({ folderMissing: t("Folder not available"), timedOut: t("Folder not available"), readOnly: t("Folder is read-only"), invalidFolder: t("Folder not available"), unsupportedFolder: t("Not supported on this system") });
  async function run(action: () => Promise<unknown>) {
    if (busy) return;
    busy = true; error = null;
    try { await action(); } catch (e) { error = errorMessage(e); } finally { busy = false; }
  }
  async function choose() {
    const epoch = session.lockEpoch;
    const token = await api.vaultLocationPick();
    if (!token) return;
    const probe = await api.vaultLocationProbe(token);
    if (epoch === session.lockEpoch && session.status.state === "unlocked") target = probe;
  }
</script>
<section>
  <h2>{t("Also sync to folder")}</h2>
  {#if settings.value?.secondaryFolder}<p class="path">{settings.value.secondaryFolder}</p>{/if}
  <p role="status">
    {#if sync.status.state === "idle" && sync.status.lastSync != null}{t("Last synced %@", [formatDate(sync.status.lastSync, "datetime")])}
    {:else if sync.status.state === "syncing"}{t("Syncing…")}
    {:else if sync.status.state === "unavailable"}{unavailable[sync.status.reason]}
    {:else if sync.status.state === "error"}{errors[sync.status.code]}
    {:else if sync.status.state === "off"}{t("Not syncing to another folder")}{/if}
  </p>
  {#if sync.status.state === "error" && sync.status.recoveryFile}
    <p role="alert" data-testid="sync-recovery-file">{t("The previous vault is kept as %@ next to the synced copy. Do not delete it.", [sync.status.recoveryFile])}</p>
  {/if}
  <Button disabled={busy || session.status.state !== "unlocked"} onclick={() => void run(choose)}>{t("Choose folder…")}</Button>
  {#if sync.status.state !== "off"}
    <Button disabled={busy || sync.status.state === "syncing" || session.status.state !== "unlocked"} onclick={() => void run(async () => { sync.status = await api.syncNow(); })}>{t("Sync now")}</Button>
    <Button disabled={busy} onclick={() => void run(async () => { await api.syncDisconnectSecondary(); sync.status = { state: "off" }; await settings.load(); })}>{t("Stop syncing")}</Button>
  {/if}
  <p>{t("Stopping sync keeps the files in that folder.")}</p>
  {#if error}<p role="alert">{error}</p>{/if}
</section>
{#if target}<LocationChangeSheet {target} secondary onclose={() => target = null} />{/if}
<style>.path { overflow-wrap: anywhere; }</style>
