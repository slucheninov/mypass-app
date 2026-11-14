<script lang="ts">
  // "Connect a browser": the three steps, the extension id, and the guide to using MyPass without
  // other password managers (ported from the old app's help sheet). It lives in the Settings window
  // because the Browser pane opens it. Links go through the opener (scope: the four help hosts of
  // `capabilities/settings.json`); chrome:// addresses are plain selectable text, so nothing here
  // needs the clipboard.
  import { api } from "../../lib/api";
  import { t } from "../../lib/i18n/index.svelte";
  import { toasts } from "../../lib/stores/toasts.svelte";
  import Button from "../../lib/ui/Button.svelte";
  import Modal from "../../lib/ui/Modal.svelte";
  import { webLinks } from "../../lib/ui/web-links";

  let { onclose }: { onclose: () => void } = $props();

  /** The id of the MyPass extension (a fixed key in its manifest). */
  const EXTENSION_ID = "hjijbabodfldconmhdcdphlbgaajkejn";

  type Section = { name: string; steps: string[]; source: string; address?: string };

  const sections: Section[] = $derived([
    {
      name: t("Chrome’s own autofill and Google Password Manager"),
      steps: [
        t("Open Chrome’s menu → Passwords and autofill → Google Password Manager → Settings, or paste the address below into that profile’s address bar."),
        t("Turn off offers to save passwords and passkeys, automatic passkey creation if shown, and automatic sign-in."),
        t("These controls do not remove saved credentials or guarantee that every autofill and passkey option is off. Review any remaining provider choices when signing in."),
      ],
      source: "https://support.google.com/chrome/answer/6197437?hl=en",
      address: "chrome://password-manager/settings",
    },
    {
      name: t("Bitwarden"),
      steps: [
        t("In the Bitwarden extension, open Settings → Autofill and look for the options that fill logins when a page loads and that offer to save logins. Turn them off."),
        t("In its passkey settings, look for the options to save passkeys and to sign in with them. Turn them off."),
        t("Wording varies between Bitwarden versions. Turning these off does not remove anything saved in Bitwarden."),
      ],
      source: "https://bitwarden.com/help/auto-fill-browser/",
    },
    {
      name: t("1Password"),
      steps: [
        t("To use only MyPass in this profile, open Chrome’s Extensions page and switch 1Password off."),
        t("If you keep 1Password enabled, its Settings → Autofill & save offers separate controls for passkey sign-in and passkey saving. Turning those off does not turn off all password features."),
      ],
      source: "https://support.1password.com/save-use-passkeys/",
      address: "chrome://extensions",
    },
    {
      name: t("iCloud Passwords"),
      steps: [
        t("Open Chrome’s Extensions page in the profile you use with MyPass."),
        t("Find iCloud Passwords and switch the extension off. Leave MyPass enabled."),
        t("This disables the Chrome extension only. macOS password and passkey providers are separate; the browser check cannot confirm their settings."),
      ],
      source: "https://support.google.com/chrome/answer/2664769?hl=en",
    },
  ]);

  async function showFolder() {
    await toasts.guard(() => api.browserShowExtensionFolder());
  }
</script>

<Modal open={true} title={t("Connect a browser")} {onclose}>
  <div class="help" use:webLinks>
    <ol class="steps" aria-label={t("Connect a browser")}>
      <li>{t("Install the extension: in chrome://extensions turn on Developer mode, choose Load unpacked and pick the extension folder, or install the store build.")}</li>
      <li>{t("Open the MyPass extension popup in your browser.")}</li>
      <li>{t("Choose Connect and type the 6-digit code that MyPass shows.")}</li>
    </ol>
    <p class="id">{t("Extension ID")}: <code>{EXTENSION_ID}</code></p>
    <p><Button onclick={() => void showFolder()}>{t("Show extension folder")}</Button></p>
    <p class="note">{t("Browsers must be restarted once after the first launch of MyPass.")}</p>

    <h3>{t("Use MyPass without other managers")}</h3>
    <p>{t("Follow these steps in each Chrome profile where you use MyPass. Turning off a manager does not delete its saved passwords or passkeys.")}</p>

    {#each sections as section (section.source)}
      <section data-testid="manager-section">
        <h4>{section.name}</h4>
        <ol>
          {#each section.steps as step, index (index)}<li>{step}</li>{/each}
        </ol>
        {#if section.address}<p><code class="address">{section.address}</code></p>{/if}
        <p><a href={section.source} aria-label={`${t("Official help")}: ${section.name}`}>{t("Official help")}</a></p>
      </section>
    {/each}

    <section>
      <h4>{t("Optional: macOS AutoFill")}</h4>
      <p>{t("For system-wide changes on current macOS, open System Settings → General → AutoFill & Passwords and review AutoFill Passwords and Passkeys. On macOS Sonoma, use System Settings → Passwords → Password Options. This affects other apps too; it is separate from disabling the Chrome extension.")}</p>
      <p><a href="https://support.apple.com/en-us/120758">{t("Apple AutoFill help")}</a></p>
    </section>

    <section>
      <h4>{t("Why passkeys conflict")}</h4>
      <p>{t("Chrome lets only one extension attach to its webAuthenticationProxy API at a time, and that is how MyPass takes passkey requests. If another password manager is already attached, or its passkey feature is on, MyPass cannot take passkey requests until that is turned off.")}</p>
    </section>

    <section>
      <h4>{t("Check the result")}</h4>
      <p>{t("Reopen the MyPass extension and choose Check this browser. Allow the read-only check when Chrome asks. Return to MyPass Settings → Browser to see the report and its receipt time.")}</p>
      <p>{t("Then retry the website’s passkey action. Confirm in MyPass and wait for the website to finish. A saved key in MyPass alone does not confirm that the website completed registration.")}</p>
    </section>
  </div>
  {#snippet footer()}
    <Button variant="primary" onclick={onclose}>{t("Done")}</Button>
  {/snippet}
</Modal>

<style>
  .help {
    min-width: 0;
  }
  h3,
  h4 {
    margin: 16px 0 6px;
    font-weight: 600;
  }
  h3 {
    font-size: 14px;
  }
  h4 {
    font-size: 13px;
  }
  p,
  ol {
    margin: 6px 0;
  }
  ol {
    padding-left: 20px;
  }
  .note {
    color: var(--text-2);
  }
  code {
    user-select: text;
  }
  a {
    color: var(--accent);
  }
</style>
