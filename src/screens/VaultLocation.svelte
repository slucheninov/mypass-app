<script lang="ts">
  import { platform } from "../lib/platform.svelte";
  import { shortVaultLocation } from "../lib/vault-location";

  /** The short location of the vault at `path`; the full path is the tooltip and the screen reader text. */
  let { path }: { path: string } = $props();
</script>

<span class="location" title={path}><span aria-hidden="true">{shortVaultLocation(path, platform.homeDir)}</span><span class="sr-only">{path}</span></span>

<style>
  .location {
    position: relative;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
