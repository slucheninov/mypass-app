<script lang="ts">
  // The settings window: a pane list on the left, the chosen pane on the right. The last pane is
  // remembered through `prefs` (layout preferences only, storage failures ignored). Every pane
  // reads and writes through the one `settings` store; nothing is rendered until it has loaded.
  import { tick } from "svelte";
  import { t } from "../lib/i18n/index.svelte";
  import { readPref, writePref } from "../lib/prefs";
  import { settings } from "../lib/stores/settings.svelte";
  import ToastHost from "../lib/ui/ToastHost.svelte";
  import About from "./settings/About.svelte";
  import Appearance from "./settings/Appearance.svelte";
  import Browser from "./settings/Browser.svelte";
  import General from "./settings/General.svelte";
  import Security from "./settings/Security.svelte";
  import Vault from "./settings/Vault.svelte";

  const PANES = ["General", "Vault", "Security", "Browser", "Appearance", "About"] as const;
  type Pane = (typeof PANES)[number];
  const PREF_KEY = "layout.settingsPane";

  function label(pane: Pane): string {
    switch (pane) {
      case "General":
        return t("General");
      case "Vault":
        return t("Vault");
      case "Security":
        return t("Security");
      case "Browser":
        return t("Browser");
      case "Appearance":
        return t("Appearance");
      case "About":
        return t("About");
    }
  }

  function remembered(): Pane {
    const stored = readPref(PREF_KEY);
    return PANES.find((p) => p === stored) ?? "General";
  }

  let current = $state<Pane>(remembered());
  let list: HTMLElement | undefined = $state();

  function choose(pane: Pane) {
    current = pane;
    writePref(PREF_KEY, pane);
  }

  async function onkeydown(event: KeyboardEvent, index: number) {
    const delta = event.key === "ArrowDown" || event.key === "ArrowRight" ? 1 : event.key === "ArrowUp" || event.key === "ArrowLeft" ? -1 : 0;
    const edge = event.key === "Home" ? 0 : event.key === "End" ? PANES.length - 1 : null;
    if (delta === 0 && edge === null) return;
    event.preventDefault();
    const next = edge !== null ? PANES[edge] : PANES[(index + delta + PANES.length) % PANES.length];
    if (!next) return;
    choose(next);
    await tick();
    list?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')?.focus();
  }
</script>

<main data-testid="settings-app" class="settings">
  <nav class="panes">
    <div role="tablist" aria-orientation="vertical" aria-label={t("Settings")} bind:this={list}>
      {#each PANES as pane, index (pane)}
        <button
          type="button"
          role="tab"
          id="settings-tab-{pane}"
          aria-selected={pane === current}
          aria-controls="settings-panel"
          tabindex={pane === current ? 0 : -1}
          onclick={() => choose(pane)}
          onkeydown={(e) => onkeydown(e, index)}
        >
          {label(pane)}
        </button>
      {/each}
    </div>
  </nav>
  <!-- Focusable: a pane may open on text only (Browser, About), and the panel must stay reachable. -->
  <div class="content" role="tabpanel" tabindex="0" id="settings-panel" aria-labelledby="settings-tab-{current}">
    <h1>{label(current)}</h1>
    {#if settings.value}
      {#if current === "General"}
        <General />
      {:else if current === "Vault"}
        <Vault />
      {:else if current === "Security"}
        <Security />
      {:else if current === "Browser"}
        <Browser />
      {:else if current === "Appearance"}
        <Appearance />
      {:else}
        <About />
      {/if}
    {/if}
  </div>
</main>
<ToastHost />

<style>
  .settings {
    display: flex;
    height: 100vh;
    background: var(--bg);
    color: var(--text);
  }
  .panes {
    flex: none;
    width: 160px;
    padding: 12px 8px;
    border-right: var(--sep);
    background: var(--sidebar-bg);
  }
  [role="tablist"] {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  [role="tab"] {
    padding: 6px 10px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: inherit;
    text-align: left;
  }
  [role="tab"][aria-selected="true"] {
    background: var(--accent);
    color: #ffffff;
  }
  .content {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 16px 24px 24px;
  }
  h1 {
    margin: 0 0 8px;
    font-size: 18px;
    font-weight: 600;
  }
</style>
