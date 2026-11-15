<script lang="ts">
  import { api, errorMessage } from "../lib/api";
  import type { FolderProbe_Serialize, MigrationStrategy } from "../lib/bindings";
  import { session } from "../lib/stores/session.svelte";
  import { t } from "../lib/i18n/index.svelte";
  import Modal from "../lib/ui/Modal.svelte";
  import Button from "../lib/ui/Button.svelte";
  let { target, secondary = false, requestedIcloud = false, onclose }: { target: FolderProbe_Serialize; secondary?: boolean; requestedIcloud?: boolean; onclose: () => void } = $props();
  let strategy = $state<MigrationStrategy>("merge");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let consumed = $state(false);
  const currentPath = session.status.vaultPath;
  const options = $derived(target.probe.exists ? [
    { value: "merge" as const, label: t("Merge both (recommended)") },
    { value: "adoptRemote" as const, label: t("Use the vault in that folder") },
    { value: "overwriteRemote" as const, label: t("Replace it with my current vault") },
  ] : [{ value: "seedRemote" as const, label: t("Copy my vault here") }]);
  $effect(() => { if (session.status.state !== "unlocked" || session.status.vaultPath !== currentPath) onclose(); });
  async function confirm() {
    if (busy || consumed || session.status.state !== "unlocked") return;
    busy = true; consumed = true;
    try {
      const selected = target.probe.exists ? strategy : "seedRemote";
      if (secondary) await api.syncConnectSecondary(target.folderToken, selected);
      else await api.vaultLocationChange({ folderToken: target.folderToken, strategy: selected });
      await session.init();
      onclose();
    } catch (e) { error = errorMessage(e); }
    finally { busy = false; }
  }
</script>
<Modal open title={secondary ? t("Also sync to folder") : t("Change vault location")} onclose={() => { if (!busy) onclose(); }} backdropCloses={!busy}>
  {#if requestedIcloud && !target.probe.insideIcloud}<p role="status">{t("This folder is outside iCloud Drive.")}</p>{/if}
  <fieldset disabled={busy || consumed}>
    <legend>{t("Choose how to use this folder")}</legend>
    {#each options as option}
      <label><input type="radio" name="migration-strategy" value={option.value} checked={target.probe.exists ? strategy === option.value : true} onchange={() => strategy = option.value} />{option.label}</label>
    {/each}
  </fieldset>
  <p>{t("Your current file is kept in Backups.")}</p>
  {#if error}<p role="alert">{error}</p><p>{t("Choose the folder again to retry.")}</p>{/if}
  {#snippet footer()}
    <Button disabled={busy} onclick={onclose}>{t("Cancel")}</Button>
    <Button variant="primary" disabled={busy || consumed || target.probe.sameAsCurrent} onclick={confirm}>{t("Confirm")}</Button>
  {/snippet}
</Modal>
<style>label { display: block; margin: 10px 0; } fieldset { border: var(--sep); border-radius: var(--radius); }</style>
