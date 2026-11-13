<script lang="ts">
  // Name, version, licence and the two addresses. The addresses are selectable text, not links: the
  // settings window has no `opener` permission (only `main` may open URLs).
  import { t } from "../../lib/i18n/index.svelte";
  import { platform } from "../../lib/platform.svelte";

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
