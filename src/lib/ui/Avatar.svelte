<script lang="ts">
  import { avatarColor, avatarLetter } from "./avatar";

  let { text, size = 28 }: { text: string; size?: number } = $props();
</script>

<span
  class="avatar"
  aria-hidden="true"
  style="width: {size}px; height: {size}px; font-size: {Math.round(size * 0.46)}px; background: {avatarColor(text)}"
>{avatarLetter(text)}</span>

<style>
  .avatar {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius);
    color: #ffffff;
    font-weight: 600;
  }
</style>
