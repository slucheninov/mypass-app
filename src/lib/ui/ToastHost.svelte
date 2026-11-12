<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import { toasts } from "../stores/toasts.svelte";
  import Toast from "./Toast.svelte";
</script>

<section class="host" aria-label={t("Notifications")}>
  {#each toasts.items as toast (toast.id)}
    <Toast {toast} ondismiss={(id) => toasts.dismiss(id)} onpause={(id) => toasts.pause(id)} onresume={(id) => toasts.resume(id)} />
  {/each}
</section>

<style>
  .host {
    position: fixed;
    right: 16px;
    bottom: 16px;
    z-index: 200;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 8px;
    pointer-events: none;
  }
  .host > :global(*) {
    pointer-events: auto;
  }
</style>
