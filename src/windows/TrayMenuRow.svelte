<script lang="ts">
  import Avatar from "../lib/ui/Avatar.svelte";
  import Icon from "../lib/ui/Icon.svelte";
  import type { IconName } from "../lib/ui/icons";

  let {
    key, label, subtitle = "", shortcut = "", icon, avatar = false,
    chevron = false, active, onclick, onhover,
  }: {
    key: string;
    label: string;
    subtitle?: string;
    shortcut?: string;
    icon?: IconName;
    avatar?: boolean;
    chevron?: boolean;
    active: boolean;
    onclick: () => void;
    onhover: () => void;
  } = $props();
  const subtitleId = $props.id();
</script>

<button
  type="button"
  role="menuitem"
  class="row"
  aria-label={label}
  aria-describedby={subtitle ? subtitleId : undefined}
  aria-haspopup={chevron ? "menu" : undefined}
  data-menu-key={key}
  data-active={active ? "" : undefined}
  {onclick}
  onfocus={onhover}
  onpointerenter={onhover}
>
  {#if icon}<span class="icon"><Icon name={icon} /></span>{/if}
  {#if avatar}<Avatar text={label} size={16} />{/if}
  <span class="labels">
    <span class="label">{label}</span>
    {#if subtitle}<span class="subtitle" id={subtitleId}>{subtitle}</span>{/if}
  </span>
  {#if shortcut}<span class="shortcut" aria-hidden="true">{shortcut}</span>{/if}
  {#if chevron}<span class="icon"><Icon name="chevron" size={14} /></span>{/if}
</button>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    height: 24px;
    padding: 0 9px;
    border: 0;
    border-radius: 4px;
    background: transparent;
    font: inherit;
    text-align: left;
    cursor: default;
  }
  .row[data-active],
  .row:focus-visible {
    background: var(--menu-accent);
    color: #fff;
    outline: none;
  }
  .labels {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .label,
  .subtitle {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .shortcut,
  .subtitle {
    color: GrayText;
  }
  .subtitle {
    font-size: 11px;
  }
  .shortcut,
  .icon {
    flex: none;
  }
  .icon {
    display: inline-flex;
  }
  .row[data-active] :is(.shortcut, .subtitle),
  .row:focus-visible :is(.shortcut, .subtitle) {
    color: inherit;
  }
</style>
