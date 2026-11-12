<script lang="ts">
  import type { IdleTimeout, SettingsPatch_Deserialize } from "../../lib/bindings";
  import { t } from "../../lib/i18n/index.svelte";
  import { platform } from "../../lib/platform.svelte";
  import { settings } from "../../lib/stores/settings.svelte";
  import Select from "../../lib/ui/Select.svelte";
  import Toggle from "../../lib/ui/Toggle.svelte";
  import { applyPatch } from "./apply";
  import DeviceUnlockSection from "./DeviceUnlockSection.svelte";
  import FormError from "./FormError.svelte";
  import Row from "./Row.svelte";

  let error = $state<string | null>(null);
  // Bumped after a rejected write so a select shows the stored value again.
  let resets = $state(0);

  function idleLabel(value: IdleTimeout): string {
    switch (value) {
      case "immediately":
        return t("Immediately");
      case "oneMinute":
        return t("1 minute");
      case "fiveMinutes":
        return t("5 minutes");
      case "fifteenMinutes":
        return t("15 minutes");
      case "thirtyMinutes":
        return t("30 minutes");
      case "oneHour":
        return t("1 hour");
      case "fourHours":
        return t("4 hours");
      case "never":
        return t("Never");
    }
  }

  const IDLE_ORDER: IdleTimeout[] = [
    "immediately",
    "oneMinute",
    "fiveMinutes",
    "fifteenMinutes",
    "thirtyMinutes",
    "oneHour",
    "fourHours",
    "never",
  ];
  const idleOptions = $derived(IDLE_ORDER.map((value) => ({ value, label: idleLabel(value) })));

  async function send(patch: SettingsPatch_Deserialize) {
    error = await applyPatch(patch);
    if (error) resets += 1;
  }
</script>

{#if settings.value}
  <Row label={t("Lock after inactivity")}>
    {#key resets}
      <Select
        value={settings.value.autoLockIdleTimeout}
        options={idleOptions}
        label={t("Lock after inactivity")}
        onchange={(autoLockIdleTimeout) => send({ autoLockIdleTimeout })}
      />
    {/key}
  </Row>
  <Row label={t("Lock when the computer sleeps")}>
    <Toggle
      checked={settings.value.lockOnDeviceSleep}
      label={t("Lock when the computer sleeps")}
      onchange={(lockOnDeviceSleep) => send({ lockOnDeviceSleep })}
    />
  </Row>
  <Row label={t("Lock when the screen locks")}>
    <Toggle
      checked={settings.value.lockOnScreenLock}
      label={t("Lock when the screen locks")}
      onchange={(lockOnScreenLock) => send({ lockOnScreenLock })}
    />
  </Row>
  {#if platform.deviceUnlockAvailable}
    <DeviceUnlockSection />
  {/if}
{/if}
<FormError message={error} />
