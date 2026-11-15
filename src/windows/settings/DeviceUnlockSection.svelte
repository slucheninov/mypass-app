<script lang="ts">
  import { onMount } from "svelte";
  import { device } from "../../lib/stores/device.svelte";
  import { session } from "../../lib/stores/session.svelte";
  import { settings } from "../../lib/stores/settings.svelte";
  import { t } from "../../lib/i18n/index.svelte";
  import { errorMessage } from "../../lib/api";
  import Button from "../../lib/ui/Button.svelte";
  import Toggle from "../../lib/ui/Toggle.svelte";
  import Select from "../../lib/ui/Select.svelte";
  import PasswordField from "../../lib/ui/PasswordField.svelte";
  import Modal from "../../lib/ui/Modal.svelte";
  import Row from "./Row.svelte";
  import type { UnlockWithDeviceWindow, RequireAccountPasswordWindow } from "../../lib/bindings";

  let field: PasswordField | undefined = $state();
  let intent = $state<{ touchId: boolean; macPassword: boolean } | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let hasPassword = $state(false);
  const deviceOptions = $derived<{ value: UnlockWithDeviceWindow; label: string }[]>([
    { value: "oneHour", label: t("For 1 hour") }, { value: "fourHours", label: t("For 4 hours") },
    { value: "eightHours", label: t("For 8 hours") }, { value: "twentyFourHours", label: t("For 24 hours") }, { value: "never", label: t("Never") },
  ]);
  const accountOptions = $derived<{ value: RequireAccountPasswordWindow; label: string }[]>([
    { value: "oneDay", label: t("After 1 day") }, { value: "sevenDays", label: t("After 7 days") },
    { value: "fourteenDays", label: t("After 14 days") }, { value: "thirtyDays", label: t("After 30 days") },
    { value: "ninetyDays", label: t("After 90 days") }, { value: "never", label: t("Never") },
  ]);
  onMount(() => {
    void device.refresh().catch(e => error = errorMessage(e));
    return session.onChange(next => { if (next !== "unlocked") close(); });
  });
  function close() { field?.clear(); hasPassword = false; intent = null; error = null; }
  $effect(() => { if (session.status.state !== "unlocked") close(); });
  async function configure(touchId: boolean, macPassword: boolean, password: string | null = null) {
    if (busy) return;
    const epoch = session.lockEpoch;
    if (session.status.state !== "unlocked") { close(); return; }
    busy = true; error = null;
    try { await device.configure(password, touchId, macPassword); if (epoch === session.lockEpoch) close(); }
    catch (e) { if (epoch === session.lockEpoch) error = errorMessage(e); }
    finally { busy = false; }
  }
  function change(touchId: boolean, macPassword: boolean) {
    error = null;
    if ((touchId || macPassword) && !device.status?.touchIdEnabled && !device.status?.macPasswordEnabled) intent = { touchId, macPassword };
    else void configure(touchId, macPassword);
  }
  async function saveWindow(patch: Parameters<typeof settings.update>[0]) {
    try { await settings.update(patch); await device.refresh(); } catch (e) { error = errorMessage(e); }
  }
</script>
<section data-testid="device-unlock-slot">
  <h2>{t("Unlock")}</h2>
  <p>{t("Unlock without typing your master password using Touch ID or your Mac password. You'll still be asked for the master password on the schedule above.")}</p>
  <Row label={t("Unlock using Touch ID")}><Toggle label={t("Unlock using Touch ID")} checked={device.status?.touchIdEnabled ?? false} disabled={busy || session.status.state !== "unlocked"} onchange={value => change(value, device.status?.macPasswordEnabled ?? false)} /></Row>
  <Row label={t("Unlock app with Mac password")}><Toggle label={t("Unlock app with Mac password")} checked={device.status?.macPasswordEnabled ?? false} disabled={busy || session.status.state !== "unlocked"} onchange={value => change(device.status?.touchIdEnabled ?? false, value)} /></Row>
  {#if settings.value}
    <Row label={t("Unlock with device:")}><Select label={t("Unlock with device:")} value={settings.value.unlockWithDeviceWindow} options={deviceOptions} onchange={unlockWithDeviceWindow => void saveWindow({ unlockWithDeviceWindow })} /></Row>
    <Row label={t("Require account password:")}><Select label={t("Require account password:")} value={settings.value.requireAccountPasswordWindow} options={accountOptions} onchange={requireAccountPasswordWindow => void saveWindow({ requireAccountPasswordWindow })} /></Row>
  {/if}
  {#if error && !intent}<p role="alert">{error}</p>{/if}
</section>
<Modal open={intent !== null} title={t("Confirm Master Password")} onclose={close}>
  <form onsubmit={event => { event.preventDefault(); if (intent && hasPassword && !busy) void configure(intent.touchId, intent.macPassword, field?.take() ?? null); }}>
    <PasswordField bind:this={field} label={t("Master password")} oninput={length => hasPassword = length > 0} />
    {#if error}<p role="alert">{error}</p>{/if}
    <Button disabled={busy} onclick={close}>{t("Cancel")}</Button>
    <Button type="submit" variant="primary" disabled={busy || !hasPassword}>{t("Enable")}</Button>
  </form>
</Modal>
