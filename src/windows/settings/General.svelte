<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../../lib/api";
  import { events, type Language, type LoginItemStatus, type SettingsPatch_Deserialize, type ShortcutKind } from "../../lib/bindings";
  import { t } from "../../lib/i18n/index.svelte";
  import { settings } from "../../lib/stores/settings.svelte";
  import Select from "../../lib/ui/Select.svelte";
  import Stepper from "../../lib/ui/Stepper.svelte";
  import Toggle from "../../lib/ui/Toggle.svelte";
  import { applyPatch, systemText } from "./apply";
  import { DebouncedNumber } from "./debounced.svelte";
  import FormError from "./FormError.svelte";
  import HotkeyRow from "./HotkeyRow.svelte";
  import Row from "./Row.svelte";

  const CLIPBOARD_MIN = 10;
  const CLIPBOARD_MAX = 120;
  const CLIPBOARD_STEP = 5;

  let error = $state<string | null>(null);
  // Bumped after a rejected write so a select shows the stored value again.
  let resets = $state(0);

  const languages = $derived<{ value: Language; label: string }[]>([
    { value: "system", label: t("System") },
    { value: "en", label: t("English") },
    { value: "uk", label: t("Ukrainian") },
    { value: "ru", label: t("Russian") },
  ]);

  async function send(patch: SettingsPatch_Deserialize): Promise<boolean> {
    error = await applyPatch(patch);
    if (error) resets += 1;
    return error === null;
  }

  // ---- launch at login: the platform status drives the row, re-read on open and after each toggle ----
  let loginStatus = $state<LoginItemStatus | null>(null);
  let loginError = $state<string | null>(null);

  async function readLoginStatus() {
    try {
      loginStatus = await api.settingsLoginItemStatus();
    } catch {
      loginStatus = "unsupported";
    }
  }

  async function toggleLogin(launchAtLogin: boolean) {
    loginError = null;
    try {
      await settings.update({ launchAtLogin });
    } catch (e) {
      loginError = t("Could not update login item: %@", [systemText(e)]);
    }
    await readLoginStatus();
  }

  // ---- hotkeys ----
  let conflicts = $state<Record<ShortcutKind, boolean>>({ lock: false, show: false });

  async function setHotkey(kind: ShortcutKind, shortcut: string) {
    conflicts[kind] = false;
    await send(kind === "lock" ? { shortcutLock: shortcut } : { shortcutShow: shortcut });
  }

  // ---- clipboard delay ----
  const clipboard = new DebouncedNumber(
    () => settings.value?.clipboardClearSeconds ?? 30,
    async (clipboardClearSeconds) => void (await send({ clipboardClearSeconds })),
  );
  // "30 s" is the unit text without its number, so the stepper reads "30" + "s".
  const secondsUnit = $derived(t("%lld s", [""]).trim());

  onMount(() => {
    void readLoginStatus();
    let off: (() => void) | null = null;
    let gone = false;
    // A conflict from a hotkey change made here (the startup ones go to the main window only).
    void events.shortcutConflictEvent
      .listen((e) => {
        conflicts[e.payload.kind] = true;
      })
      .then((unlisten) => {
        if (gone) unlisten();
        else off = unlisten;
      });
    return () => {
      gone = true;
      off?.();
      clipboard.flush();
    };
  });
</script>

{#if settings.value}
  <Row label={t("Language")}>
    {#key resets}
      <Select
        value={settings.value.language}
        options={languages}
        label={t("Language")}
        onchange={(language) => send({ language })}
      />
    {/key}
  </Row>

  {#if loginStatus !== null && loginStatus !== "unsupported"}
    <Row label={t("Launch at login")}>
      <Toggle checked={loginStatus === "enabled"} label={t("Launch at login")} onchange={toggleLogin} />
      {#snippet hint()}
        {#if loginStatus === "requiresApproval"}
          {t("Approval required. Open System Settings > General > Login Items and allow MyPass.")}
        {/if}
        {#if loginError}<span class="problem" role="alert">{loginError}</span>{/if}
      {/snippet}
    </Row>
  {/if}

  <Row label={t("Show menu bar icon")}>
    <Toggle
      checked={settings.value.showMenuBarExtra}
      label={t("Show menu bar icon")}
      onchange={(showMenuBarExtra) => send({ showMenuBarExtra })}
    />
  </Row>

  <HotkeyRow
    label={t("Lock hotkey")}
    value={settings.value.shortcutLock}
    conflict={conflicts.lock}
    onrecord={(shortcut) => setHotkey("lock", shortcut)}
    onclear={() => setHotkey("lock", "")}
  />
  <HotkeyRow
    label={t("Show hotkey")}
    value={settings.value.shortcutShow}
    conflict={conflicts.show}
    onrecord={(shortcut) => setHotkey("show", shortcut)}
    onclear={() => setHotkey("show", "")}
  />

  <Row label={t("Clear clipboard after")}>
    <Stepper
      value={clipboard.value}
      min={CLIPBOARD_MIN}
      max={CLIPBOARD_MAX}
      step={CLIPBOARD_STEP}
      label={t("Clear clipboard after")}
      unit={secondsUnit}
      onchange={(v) => {
        error = null;
        clipboard.set(v);
      }}
    />
  </Row>
{/if}
<FormError message={error} />

<style>
  .problem {
    display: block;
    color: var(--danger);
  }
</style>
