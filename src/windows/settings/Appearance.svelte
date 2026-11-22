<script lang="ts">
  import type { AppIcon, Theme } from "../../lib/bindings";
  import { t } from "../../lib/i18n/index.svelte";
  import { settings } from "../../lib/stores/settings.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import { applyPatch } from "./apply";
  import FormError from "./FormError.svelte";
  import Row from "./Row.svelte";

  let error = $state<string | null>(null);

  const themes = $derived<{ value: Theme; label: string }[]>([
    { value: "system", label: t("System") },
    { value: "light", label: t("Light") },
    { value: "dark", label: t("Dark") },
  ]);

  async function chooseIcon(appIcon: AppIcon) {
    error = await applyPatch({ appIcon });
  }

  async function choose(theme: Theme) {
    error = await applyPatch({ theme });
  }
</script>

{#if settings.value}
  <Row label={t("Theme")}>
    <Segmented value={settings.value.theme} options={themes} label={t("Theme")} onchange={choose} />
  </Row>
  <Row label={t("App icon")}>
    <Segmented value={settings.value.appIcon} options={themes} label={t("App icon")} onchange={chooseIcon} />
  </Row>
{/if}
<FormError message={error} />
