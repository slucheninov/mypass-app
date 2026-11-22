<script lang="ts">
  // A small mark for a social sign-in provider: its initial on the provider's colour. Decorative;
  // the caller puts the provider name next to it or in an aria-label.
  import type { SignInProvider } from "../bindings";

  let { provider, size = 14 }: { provider: SignInProvider; size?: number } = $props();

  const MARK: Record<SignInProvider, { letter: string; bg: string }> = {
    google: { letter: "G", bg: "#4285f4" },
    facebook: { letter: "f", bg: "#1877f2" },
    apple: { letter: "A", bg: "#1d1d1f" },
    microsoft: { letter: "M", bg: "#737373" },
    github: { letter: "G", bg: "#24292f" },
  };
</script>

<span
  class="mark"
  aria-hidden="true"
  data-provider={provider}
  style:width="{size}px"
  style:height="{size}px"
  style:font-size="{Math.round(size * 0.64)}px"
  style:background={MARK[provider].bg}>{MARK[provider].letter}</span
>

<style>
  .mark {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    color: #fff;
    font-weight: 700;
    line-height: 1;
  }
</style>
