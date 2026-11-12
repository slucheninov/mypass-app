<script lang="ts">
  // Draws the open-guard warnings one at a time. Mounted once in the main window; it registers
  // itself as the presenter `confirmOpen` talks to. Closing the host declines a pending warning.
  import { setWarningPresenter } from "../lib/open-guard";
  import type { OpenWarning } from "../lib/bindings";
  import OpenWarningDialog from "./OpenWarningDialog.svelte";

  let current = $state<OpenWarning | null>(null);
  let resolver: ((proceed: boolean) => void) | null = null;

  function decide(proceed: boolean): void {
    const resolve = resolver;
    resolver = null;
    current = null;
    resolve?.(proceed);
  }

  $effect(() => {
    setWarningPresenter(
      (warning) =>
        new Promise<boolean>((resolve) => {
          resolver?.(false);
          resolver = resolve;
          current = warning;
        }),
    );
    return () => {
      setWarningPresenter(null);
      decide(false);
    };
  });
</script>

{#if current}
  <OpenWarningDialog warning={current} ondecide={decide} />
{/if}
