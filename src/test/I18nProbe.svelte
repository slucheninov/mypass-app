<script lang="ts">
  // Test-only probe: renders one translated string so tests can watch it change with the language.
  import { t } from "../lib/i18n/index.svelte";
  let { text }: { text: string } = $props();
</script>

<p data-testid="probe">{t(text)}</p>
