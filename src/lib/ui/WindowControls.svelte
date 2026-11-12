<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import { t } from "../i18n/index.svelte";

  let { os }: { os: "macos" | "windows" | "linux" } = $props();

  let maximized = $state(false);

  async function refresh() {
    try {
      maximized = await getCurrentWindow().isMaximized();
    } catch {
      // The window API is unavailable outside Tauri; keep the last known state.
    }
  }

  async function run(action: () => Promise<unknown>) {
    try {
      await action();
    } catch {
      // A refused window call is not actionable by the user.
    }
  }

  // Snapping, a double-click on the bar or a keyboard shortcut change the state without a click
  // here, so the label follows every window resize.
  onMount(() => {
    if (os !== "macos") void refresh();
  });

  async function toggleMaximize() {
    const win = getCurrentWindow();
    await run(() => (maximized ? win.unmaximize() : win.maximize()));
    await refresh();
  }
</script>

<svelte:window onresize={() => (os !== "macos" ? void refresh() : undefined)} />

{#if os === "macos"}
  <!-- Native traffic lights sit on the left; this spacer keeps toolbar content clear of them. -->
  <div class="inset" data-testid="titlebar-inset" style="width: 72px"></div>
{:else}
  <div class="bar" data-tauri-drag-region>
    <button type="button" aria-label={t("Minimize")} title={t("Minimize")} onclick={() => run(() => getCurrentWindow().minimize())}>
      <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true" focusable="false"><path d="M0 5h10" stroke="currentColor" fill="none" /></svg>
    </button>
    <button
      type="button"
      aria-label={maximized ? t("Restore") : t("Maximize")}
      title={maximized ? t("Restore") : t("Maximize")}
      onclick={toggleMaximize}
    >
      <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true" focusable="false">
        {#if maximized}
          <path d="M2.5 2.5h7v7h-7z M.5 7.5v-7h7" stroke="currentColor" fill="none" />
        {:else}
          <path d="M.5 .5h9v9h-9z" stroke="currentColor" fill="none" />
        {/if}
      </svg>
    </button>
    <button type="button" class="close" aria-label={t("Close")} title={t("Close")} onclick={() => run(() => getCurrentWindow().close())}>
      <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true" focusable="false"><path d="M0 0l10 10M10 0L0 10" stroke="currentColor" fill="none" /></svg>
    </button>
  </div>
{/if}

<style>
  .inset {
    flex: none;
    height: 100%;
  }
  .bar {
    display: flex;
    align-items: stretch;
    height: 100%;
    margin-left: auto;
  }
  button {
    width: 46px;
    border: 0;
    background: transparent;
    color: var(--text);
  }
  button:hover {
    background: var(--hover);
  }
  .close:hover {
    background: var(--danger-hover);
    color: var(--danger-hover-text);
  }
</style>
