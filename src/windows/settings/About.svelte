<script lang="ts">
  // Name, version, licence and the two addresses. The addresses are selectable text, not links: the
  // settings window has no `opener` permission (only `main` may open URLs).
  import { update } from "../../lib/stores/update.svelte";
  import { settings } from "../../lib/stores/settings.svelte";
  import { toasts } from "../../lib/stores/toasts.svelte";
  import { i18n } from "../../lib/i18n/index.svelte";
  import type { UpdateFailure } from "../../lib/bindings";
  import Button from "../../lib/ui/Button.svelte";
  import Toggle from "../../lib/ui/Toggle.svelte";
  import Row from "./Row.svelte";
  import { t } from "../../lib/i18n/index.svelte";
  import { platform } from "../../lib/platform.svelte";

  function relativeTime(at: number): string {
    const seconds = (at - Date.now()) / 1000;
    const unit = Math.abs(seconds) < 3600 ? "minute" : Math.abs(seconds) < 86400 ? "hour" : "day";
    const divisor = unit === "minute" ? 60 : unit === "hour" ? 3600 : 86400;
    return new Intl.RelativeTimeFormat(i18n.lang, { numeric: "auto" }).format(Math.round(seconds / divisor), unit);
  }
  const failures = $derived<Record<UpdateFailure, string>>({ offline: t("Could not reach GitHub."), rateLimited: t("GitHub rate limit reached; try again later."), badStatus: t("GitHub answered with an error."), unexpected: t("GitHub returned an unexpected answer."), unknownVersion: t("Cannot determine the running version.") });
  const status = $derived(update.status.status);
  const REPOSITORY = "https://github.com/slucheninov/mypass-app";
  const NOTICES = "https://github.com/slucheninov/mypass-app/blob/main/THIRD_PARTY_NOTICES.md";
</script>

<section class="about">
  <p class="name">{"MyPass"}</p>
  {#if platform.version}<p>{t("Version %@", [platform.version])}</p>{/if}
  <p>{t("Licensed under Apache-2.0")}</p>
  <p class="address">{REPOSITORY}</p>
  <p class="label">{t("Third-party notices")}</p>
  <p class="address">{NOTICES}</p>
<Button disabled={status.state === "checking"} onclick={() => void update.check()}>{t("Check for Updates")}</Button>
  <p role="status">
    {#if status.state === "failed"}{failures[status.kind]}
    {:else if status.state === "checking"}{t("Checking…")}
    {:else if status.state === "upToDate"}{t("Up to date")}{#if status.checkedAt != null} · {t("Checked %@", [relativeTime(status.checkedAt)])}{/if}
    {:else if status.state === "available"}{t("MyPass %@ available…", [status.version])}{/if}
  </p>
  {#if status.state === "available" || update.status.banner}<Button onclick={() => void update.open()}>{t("View release")}</Button>{/if}
  {#if settings.value}<Row label={t("Check for updates automatically")}><Toggle checked={settings.value.updateCheckEnabled} label={t("Check for updates automatically")} onchange={updateCheckEnabled => void settings.update({ updateCheckEnabled }).catch(e => toasts.reportError(e))} /></Row>{/if}
  <p>{t("Only the app version is sent to GitHub.")}</p>
</section>

<style>
  .about p {
    margin: 4px 0;
  }
  .name {
    font-size: 16px;
    font-weight: 600;
  }
  .label {
    margin-top: 12px;
    color: var(--text-2);
  }
  .address {
    color: var(--text-2);
    word-break: break-all;
    user-select: text;
  }
</style>
