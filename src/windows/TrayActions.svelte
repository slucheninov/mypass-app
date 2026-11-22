<script lang="ts">
  import { api } from "../lib/api";
  import type { EntrySummary_Serialize, FieldRef } from "../lib/bindings";
  import { t } from "../lib/i18n/index.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import TrayMenuRow from "./TrayMenuRow.svelte";

  let { entry, highlight, onback, ondone, onhover, isCurrent }: {
    entry: EntrySummary_Serialize;
    highlight: string | null;
    onback: () => void;
    ondone: () => void;
    onhover: (key: string) => void;
    isCurrent: () => boolean;
  } = $props();

  let copying = false;
  async function copy(kind: "username" | "password" | "totp"): Promise<void> {
    if (copying || !isCurrent()) return;
    copying = true;
    try {
      const field: FieldRef = { kind };
      await api.entryCopy(entry.id, field);
      // A result from a dismissed view must not close a newly opened view.
      if (isCurrent()) ondone();
    } catch (e) {
      if (isCurrent()) toasts.reportError(e);
    } finally {
      copying = false;
    }
  }
</script>

<TrayMenuRow key="back" label={t("Back")} icon="chevron-left" active={highlight === "back"} onclick={onback} onhover={() => onhover("back")} />
{#if entry.hasUsername}
  <TrayMenuRow key="username" label={t("Copy user name")} active={highlight === "username"} onclick={() => void copy("username")} onhover={() => onhover("username")} />
{/if}
{#if entry.hasPassword}
  <TrayMenuRow key="password" label={t("Copy password")} active={highlight === "password"} onclick={() => void copy("password")} onhover={() => onhover("password")} />
{/if}
{#if entry.hasTotp}
  <TrayMenuRow key="totp" label={t("Copy code")} active={highlight === "totp"} onclick={() => void copy("totp")} onhover={() => onhover("totp")} />
{/if}
