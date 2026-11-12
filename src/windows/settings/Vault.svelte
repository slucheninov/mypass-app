<script lang="ts">
  // The current vault (status only: the toolbar owns opening another one) and its backups: how many
  // to keep, how often, the list with Restore (confirmed first) and a button that reveals the folder.
  // The reveal label is the one place the UI names the file manager.
  import { onMount } from "svelte";
  import { api, ApiFailure, errorMessage } from "../../lib/api";
  import type { BackupInterval, BackupStatus_Serialize } from "../../lib/bindings";
  import { formatBytes, formatDate } from "../../lib/i18n/format";
  import { t } from "../../lib/i18n/index.svelte";
  import { platform } from "../../lib/platform.svelte";
  import { session } from "../../lib/stores/session.svelte";
  import { settings } from "../../lib/stores/settings.svelte";
  import { connectedInfo, statusKind, statusLabel } from "../../screens/vault-status";
  import Button from "../../lib/ui/Button.svelte";
  import Modal from "../../lib/ui/Modal.svelte";
  import Select from "../../lib/ui/Select.svelte";
  import Stepper from "../../lib/ui/Stepper.svelte";
  import { applyPatch } from "./apply";
  import { DebouncedNumber } from "./debounced.svelte";
  import FormError from "./FormError.svelte";
  import Row from "./Row.svelte";

  const COUNT_MIN = 1;
  const COUNT_MAX = 50;

  let error = $state<string | null>(null);
  let resets = $state(0);
  let backups = $state<BackupStatus_Serialize | null>(null);
  let restoring = $state<string | null>(null);
  let busy = false;

  const kind = $derived(statusKind(session.status.fileStatus));
  const info = $derived(connectedInfo(session.status.fileStatus));

  const intervals = $derived<{ value: BackupInterval; label: string }[]>([
    { value: "everySave", label: t("Every save") },
    { value: "tenMinutes", label: t("Every 10 minutes") },
    { value: "hourly", label: t("Hourly") },
    { value: "daily", label: t("Daily") },
  ]);

  const revealLabel = $derived(platform.os === "windows" ? t("Show in Explorer") : t("Show in Finder"));

  async function loadBackups() {
    try {
      backups = await api.backupList();
    } catch (e) {
      backups = null;
      // No vault or a locked one simply has no list to show.
      if (!(e instanceof ApiFailure && (e.code === "noVault" || e.code === "locked"))) error = errorMessage(e);
    }
  }

  async function send(patch: Parameters<typeof applyPatch>[0]) {
    error = await applyPatch(patch);
    if (error) resets += 1;
    else await loadBackups();
  }

  const count = new DebouncedNumber(
    () => settings.value?.backupCount ?? 10,
    async (backupCount) => void (await send({ backupCount })),
  );

  async function reveal() {
    try {
      await api.appRevealBackups();
    } catch (e) {
      error = errorMessage(e);
    }
  }

  async function restore() {
    const id = restoring;
    if (id === null || busy) return;
    busy = true;
    try {
      await api.backupRestore(id);
      error = null;
    } catch (e) {
      // A backup made under another master password cannot be opened: the backend says `invalidInput`.
      error =
        e instanceof ApiFailure && e.code === "invalidInput"
          ? t("The backup does not open with your current master password")
          : errorMessage(e);
    } finally {
      busy = false;
      restoring = null;
    }
    await loadBackups();
  }

  const dash = "—";

  onMount(() => {
    void loadBackups();
    return () => count.flush();
  });
</script>

<section class="card" aria-label={t("Vault")}>
  <p class="name">{session.status.vaultName ?? dash}</p>
  {#if session.status.vaultPath}<p class="path">{session.status.vaultPath}</p>{/if}
  {#if kind}
    <p class="status">
      <span>{statusLabel(kind)}</span>
      {#if info && info.size !== null}<span>{formatBytes(info.size)}</span>{/if}
      {#if info && info.modified}<span>{formatDate(info.modified, "datetime")}</span>{/if}
    </p>
  {/if}
</section>

<h2>{t("Backups")}</h2>
{#if settings.value}
  <Row label={t("Keep")}>
    <Stepper
      value={count.value}
      min={COUNT_MIN}
      max={COUNT_MAX}
      step={1}
      label={t("Keep")}
      onchange={(v) => {
        error = null;
        count.set(v);
      }}
    />
  </Row>
  <Row label={t("Backup interval")}>
    {#key resets}
      <Select
        value={settings.value.backupInterval}
        options={intervals}
        label={t("Backup interval")}
        onchange={(backupInterval) => send({ backupInterval })}
      />
    {/key}
  </Row>
{/if}

{#if backups?.lastFailure}
  <p class="banner" role="status">
    {t("Last backup failed")}{#if backups.lastFailure.at !== null}{": "}{formatDate(backups.lastFailure.at, "datetime")}{/if}
  </p>
{/if}

{#if backups && backups.items.length > 0}
  <ul class="list">
    {#each backups.items as item (item.id)}
      <li>
        <span class="when">{item.created === null ? dash : formatDate(item.created, "datetime")}</span>
        <span class="size">{item.size === null ? dash : formatBytes(item.size)}</span>
        <Button onclick={() => (restoring = item.id)}>{t("Restore…")}</Button>
      </li>
    {/each}
  </ul>
{/if}

<p class="reveal"><Button onclick={reveal}>{revealLabel}</Button></p>
<FormError message={error} />

<Modal open={restoring !== null} title={t("Restore this backup?")} onclose={() => (restoring = null)}>
  <p>{t("Restore this backup? Your current vault is backed up first.")}</p>
  {#snippet footer()}
    <Button onclick={() => (restoring = null)}>{t("Cancel")}</Button>
    <Button variant="primary" onclick={restore}>{t("Restore")}</Button>
  {/snippet}
</Modal>

<style>
  .card {
    padding: 10px 12px;
    border: var(--sep);
    border-radius: var(--radius);
  }
  .card p {
    margin: 2px 0;
  }
  .name {
    font-weight: 600;
  }
  .status span + span::before {
    content: "\00b7";
    margin: 0 6px;
  }
  .path,
  .status {
    color: var(--text-2);
    word-break: break-all;
  }
  h2 {
    margin: 18px 0 4px;
    font-size: 14px;
    font-weight: 600;
  }
  .banner {
    margin: 10px 0;
    padding: 8px 10px;
    border: 1px solid var(--danger);
    border-radius: var(--radius);
    color: var(--danger);
  }
  .list {
    margin: 10px 0;
    padding: 0;
    list-style: none;
  }
  li {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 0;
    border-bottom: var(--sep);
  }
  .when {
    flex: 1;
  }
  .size {
    color: var(--text-2);
  }
  .reveal {
    margin: 12px 0 0;
  }
</style>
