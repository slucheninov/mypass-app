<script lang="ts">
  // Settings -> Browser: the bridge state, the connected browsers with their last diagnostics
  // report, and the native messaging host of every supported browser. The status comes from the
  // `browser` store (refreshed on mount and on every `BridgeChangedEvent`); it never holds the
  // pairing code, which belongs to the main window's sheet.
  import { onMount } from "svelte";
  import { api } from "../../lib/api";
  import type { BrowserDiagnosticsDto, PairedBrowserDto_Serialize } from "../../lib/bindings";
  import { formatDate } from "../../lib/i18n/format";
  import { t } from "../../lib/i18n/index.svelte";
  import { browser } from "../../lib/stores/browser.svelte";
  import { toasts } from "../../lib/stores/toasts.svelte";
  import Button from "../../lib/ui/Button.svelte";
  import Modal from "../../lib/ui/Modal.svelte";
  import BrowserHelpSheet from "./BrowserHelpSheet.svelte";

  onMount(() => browser.watchStatus());

  let helping = $state(false);
  let revoking = $state<PairedBrowserDto_Serialize | null>(null);
  let resetting = $state(false);
  let busy = $state(false);

  const status = $derived(browser.status);

  function passkeyLine(d: BrowserDiagnosticsDto): string {
    switch (d.proxyState) {
      case "active":
        return t("MyPass was ready to receive passkey requests at the last check.");
      case "disabled":
        return t("MyPass passkeys were turned off at the last check.");
      case "locked":
        return t("Unlock MyPass, then reopen its browser extension.");
      case "unpaired":
        return t("Connect this browser to MyPass again.");
      case "attachment-error":
        return t("MyPass could not handle passkeys. Disable other passkey extensions, then check again.");
      default:
        return t("Open MyPass and reopen its browser extension to check passkeys.");
    }
  }

  function savingLine(d: BrowserDiagnosticsDto): string {
    if (d.passwordSavingEnabled === true) return t("Browser password saving: on");
    if (d.passwordSavingEnabled === false) return t("Browser password saving: off");
    return t("Browser password saving: unknown");
  }

  function managerLine(d: BrowserDiagnosticsDto): string {
    if (d.permissionState === "not-granted") {
      return t("Check not allowed yet. In the MyPass extension, choose Check this browser and allow the check.");
    }
    if (d.permissionState !== "granted") {
      return t("Browser check permission could not be read. Reopen the extension and try again.");
    }
    if (d.extensionScanState === "not-checked") {
      return t("Extension check has not run. Choose Check this browser in the MyPass extension.");
    }
    if (d.extensionScanState !== "complete") {
      return t("Installed extensions could not be checked. Check them manually in your browser.");
    }
    return d.enabledManagers.length === 0
      ? t("No recognized competing extensions were enabled at the last check.")
      : t("Enabled managers: %@. Disable them in this browser profile.", [d.enabledManagers.join(", ")]);
  }

  function registrationLabel(state: string): string {
    switch (state) {
      case "ok":
        return t("OK");
      case "missing":
        return t("Missing");
      case "stale":
        return t("Out of date");
      default:
        return t("Error");
    }
  }

  async function revoke() {
    const target = revoking;
    if (!target || busy) return;
    busy = true;
    try {
      await api.browserRevoke(target.id);
      revoking = null;
    } catch {
      toasts.push("error", t("Could not revoke this browser. Try again."));
    } finally {
      busy = false;
    }
    await browser.refresh();
  }

  async function repair() {
    if (busy) return;
    busy = true;
    try {
      const registrations = await api.browserReinstallHost();
      if (browser.status) browser.status = { ...browser.status, registrations };
    } catch (e) {
      toasts.reportError(e);
    } finally {
      busy = false;
    }
  }

  async function reset() {
    if (busy) return;
    busy = true;
    try {
      await api.browserResetPairings();
      resetting = false;
    } catch (e) {
      toasts.reportError(e);
    } finally {
      busy = false;
    }
    await browser.refresh();
  }
</script>

{#if browser.failed && !status}
  <div class="notice" role="alert">
    <p>{t("Browser status could not be read.")}</p>
    <Button onclick={() => void browser.refresh()}>{t("Retry")}</Button>
  </div>
{/if}

{#if status}
  <div class="header">
    <p class="state" data-testid="bridge-state">{status.listening ? t("Listening") : t("Not running")}</p>
    <div class="buttons">
      <Button onclick={() => (helping = true)}>{t("Connect a browser")}</Button>
      <Button onclick={() => void toasts.guard(() => api.browserShowExtensionFolder())}>{t("Show extension folder")}</Button>
    </div>
  </div>

  {#if status.pairingStoreFailed}
    <div class="notice" role="alert">
      <p>{t("The list of connected browsers could not be read, so browser integration is not running.")}</p>
      <Button variant="danger" onclick={() => (resetting = true)}>{t("Reset paired browsers")}</Button>
    </div>
  {/if}

  <h2>{t("Connected browsers")}</h2>
  {#if status.paired.length === 0}
    <p class="empty">{t("No browsers connected yet")}</p>
  {:else}
    <ul class="browsers">
      {#each status.paired as paired (paired.id)}
        <li>
          <div class="line">
            <div>
              <strong>{paired.label}</strong>
              {#if paired.createdAt !== null}
                <span class="muted">{t("Connected %@", [formatDate(paired.createdAt, "datetime")])}</span>
              {/if}
            </div>
            <Button variant="danger" aria-label={`${t("Revoke")}: ${paired.label}`} onclick={() => (revoking = paired)}>
              {t("Revoke")}
            </Button>
          </div>
          {#if paired.diagnostics}
            <ul class="diagnostics">
              <li>{passkeyLine(paired.diagnostics)}</li>
              <li>{savingLine(paired.diagnostics)}</li>
              <li>{managerLine(paired.diagnostics)}</li>
              {#if paired.diagnostics.receivedAt !== null}
                <li class="muted">{t("Last checked")}: {formatDate(paired.diagnostics.receivedAt, "datetime")}</li>
              {/if}
            </ul>
          {:else}
            <p class="muted">{t("No report received in this app session. Open this browser’s MyPass extension and choose Check this browser.")}</p>
          {/if}
        </li>
      {/each}
    </ul>
    <p class="muted">{t("This is the last reported state, not a live check. Reopen the extension and choose Check this browser after changing another manager.")}</p>
  {/if}

  <h2>{t("Native host")}</h2>
  {#if status.registrations.length === 0}
    <p class="empty">{t("No supported browsers found")}</p>
  {:else}
    <ul class="registrations">
      {#each status.registrations as registration (registration.id)}
        <li>
          <span>{registration.display}</span>
          <span class="chip {registration.state}" data-testid="registration-state">{registrationLabel(registration.state)}</span>
        </li>
      {/each}
    </ul>
  {/if}
  <p><Button disabled={busy} onclick={() => void repair()}>{t("Repair")}</Button></p>
{/if}

{#if helping}
  <BrowserHelpSheet onclose={() => (helping = false)} />
{/if}

{#if revoking}
  <Modal open={true} title={t("Revoke this browser?")} onclose={() => (revoking = null)}>
    <p>{t("The browser will need to be connected again.")}</p>
    {#snippet footer()}
      <Button onclick={() => (revoking = null)}>{t("Cancel")}</Button>
      <Button variant="danger" disabled={busy} onclick={() => void revoke()}>{t("Revoke")}</Button>
    {/snippet}
  </Modal>
{/if}

{#if resetting}
  <Modal open={true} title={t("Reset paired browsers?")} onclose={() => (resetting = false)}>
    <p>{t("Every browser will need to be connected again.")}</p>
    {#snippet footer()}
      <Button onclick={() => (resetting = false)}>{t("Cancel")}</Button>
      <Button variant="danger" disabled={busy} onclick={() => void reset()}>{t("Reset")}</Button>
    {/snippet}
  </Modal>
{/if}

<style>
  .header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }
  .state {
    margin: 0;
    font-weight: 600;
  }
  .buttons {
    display: flex;
    gap: 8px;
  }
  h2 {
    margin: 20px 0 6px;
    font-size: 14px;
    font-weight: 600;
  }
  .notice {
    margin: 12px 0;
    padding: 8px 12px;
    border: 1px solid var(--danger);
    border-radius: var(--radius);
  }
  .notice p {
    margin: 0 0 8px;
  }
  .empty,
  .muted {
    color: var(--text-2);
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .browsers > li {
    padding: 9px 0;
    border-bottom: var(--sep);
  }
  .line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .diagnostics {
    margin-top: 4px;
    font-size: 12px;
    color: var(--text-2);
  }
  .registrations li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 0;
    border-bottom: var(--sep);
  }
  .chip {
    padding: 1px 8px;
    border-radius: 10px;
    border: 1px solid var(--control-border);
    font-size: 12px;
  }
  .chip.ok {
    color: var(--text);
  }
  .chip.missing,
  .chip.stale,
  .chip.error {
    color: var(--danger);
    border-color: var(--danger);
  }
</style>
