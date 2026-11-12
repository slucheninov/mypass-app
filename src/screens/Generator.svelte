<script lang="ts">
  // The password generator, in two shapes of one component: a popover under the editor's password
  // field (`onuse` present: the Use button hands the value to the form) and the toolbar's dialog
  // (`onuse` absent). The generated value is a display value by contract: it lives in this
  // component's state only, is dropped on lock, when the window hides and when the component is
  // destroyed, and reaches the clipboard only through the backend. Generating never records
  // anything; copying with a website does (the backend writes the history first). The history
  // list is whatever the backend holds: only ids, hosts and dates are kept here, a password only
  // while revealed (one at a time, through the shared reveal group).
  import { onDestroy, onMount } from "svelte";
  import { api, errorMessage } from "../lib/api";
  import type { GeneratorHistoryItem, RulesSummary_Serialize } from "../lib/bindings";
  import { formatDate } from "../lib/i18n/format";
  import { t } from "../lib/i18n/index.svelte";
  import { lockGuard } from "../lib/lock-guard";
  import { createRevealGroup, onWindowHidden, type Revealed } from "../lib/secret.svelte";
  import { session } from "../lib/stores/session.svelte";
  import { settings } from "../lib/stores/settings.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import Button from "../lib/ui/Button.svelte";
  import TextField from "../lib/ui/TextField.svelte";
  import Toggle from "../lib/ui/Toggle.svelte";
  import { announceCopied } from "./detail-actions";

  let {
    host = "",
    recordUrl = null,
    onuse,
  }: {
    /** The website the password is for; prefills "For website" and selects the site rules. */
    host?: string;
    /** The full page URL behind `host`; recorded instead of the bare host while the website is unchanged. */
    recordUrl?: string | null;
    onuse?: (value: string) => void;
  } = $props();

  const MIN = 8;
  const MAX = 64;
  type ClassName = "lowercase" | "uppercase" | "digits" | "symbols";
  const CLASSES: ClassName[] = ["lowercase", "uppercase", "digits", "symbols"];
  const MASK = "•".repeat(12);

  const remembered = settings.value?.generator;
  // svelte-ignore state_referenced_locally
  let length = $state(remembered?.length ?? 20);
  // svelte-ignore state_referenced_locally
  let chosen = $state<Record<ClassName, boolean>>({
    lowercase: remembered?.lowercase ?? true,
    uppercase: remembered?.uppercase ?? true,
    digits: remembered?.digits ?? true,
    symbols: remembered?.symbols ?? true,
  });
  // svelte-ignore state_referenced_locally
  let website = $state(host);
  let rules = $state<RulesSummary_Serialize | null>(null);
  let value = $state<string | null>(null);
  let sentLength = $state(0);
  let effectiveLength = $state(0);
  let error = $state<string | null>(null);
  let busy = $state(false);
  /** The number field's text while it is being typed; null shows the clamped length. */
  let draft = $state<string | null>(null);
  let tab = $state<"generate" | "history">("generate");
  let history = $state<GeneratorHistoryItem[] | null>(null);

  let ruleSeq = 0;
  let genSeq = 0;
  let historySeq = 0;
  const group = createRevealGroup();
  const revealers = new Map<string, Revealed>();

  const site = $derived(website.trim());
  const lowest = $derived(Math.max(MIN, rules?.minLength ?? MIN));
  const highest = $derived(Math.min(MAX, rules?.maxLength ?? MAX));
  // The slider is clamped to what the site allows; the chosen length itself is kept (and sent), so
  // clearing the website restores it. The backend clamps authoritatively.
  const shownLength = $derived(Math.min(highest, Math.max(lowest, length)));
  const forced = (name: ClassName) => rules?.forcedOn.includes(name) ?? false;
  const excluded = (name: ClassName) => rules?.excluded.includes(name) ?? false;
  const isOn = (name: ClassName) => forced(name) || (chosen[name] && !excluded(name));
  const locked = (name: ClassName) => forced(name) || excluded(name);
  const anyOn = $derived(CLASSES.some(isOn));

  const classLabel = (name: ClassName) =>
    name === "lowercase" ? t("Lowercase") : name === "uppercase" ? t("Uppercase") : name === "digits" ? t("Digits") : t("Symbols");

  const requirements = $derived.by(() => {
    if (!rules) return "";
    const parts: string[] = [];
    const { minLength, maxLength, maxConsecutive } = rules;
    if (minLength != null && maxLength != null) parts.push(t("%lld to %lld characters", [minLength, maxLength]));
    else if (minLength != null) parts.push(t("At least %lld characters", minLength));
    else if (maxLength != null) parts.push(t("At most %lld characters", maxLength));
    if (maxConsecutive != null) parts.push(t("At most %lld identical characters in a row", maxConsecutive));
    for (const name of rules.forcedOn) if (CLASSES.includes(name as ClassName)) parts.push(classLabel(name as ClassName));
    return parts.join(", ");
  });

  /** Runs of one character class, so digits and symbols can be coloured (decorative only). */
  const segments = $derived.by(() => {
    const out: { cls: "letter" | "digit" | "symbol"; text: string }[] = [];
    for (const ch of Array.from(value ?? "")) {
      const cls = /\p{L}/u.test(ch) ? "letter" : /\p{Nd}/u.test(ch) ? "digit" : "symbol";
      const last = out[out.length - 1];
      if (last && last.cls === cls) last.text += ch;
      else out.push({ cls, text: ch });
    }
    return out;
  });

  // Site rules follow the website field; a slower answer for an older text is dropped.
  $effect(() => {
    const text = site;
    const mine = ++ruleSeq;
    if (text === "") {
      rules = null;
      return;
    }
    api
      .generatorRulesSummary(text)
      .then((found) => {
        if (mine === ruleSeq) rules = found;
      })
      .catch(() => {
        if (mine === ruleSeq) rules = null;
      });
  });

  // The remembered defaults may load or change (another window) after mount.
  $effect(() => {
    const g = settings.value?.generator;
    if (!g) return;
    length = g.length;
    chosen = { lowercase: g.lowercase, uppercase: g.uppercase, digits: g.digits, symbols: g.symbols };
    draft = null;
  });

  function clearSecrets() {
    genSeq++;
    busy = false;
    value = null;
    error = null;
    group.hideAll();
  }

  onMount(() => {
    const offLock = session.onLocked(() => {
      clearSecrets();
      history = null;
      historySeq++;
      tab = "generate";
    });
    const offWindow = onWindowHidden(clearSecrets);
    return () => {
      offLock();
      offWindow();
    };
  });

  onDestroy(() => {
    genSeq++;
    historySeq++;
    ruleSeq++;
    value = null;
    history = null;
    group.destroy();
    revealers.clear();
  });

  function persist() {
    void settings
      .update({ generator: { length, lowercase: chosen.lowercase, uppercase: chosen.uppercase, digits: chosen.digits, symbols: chosen.symbols } })
      .catch((e) => toasts.reportError(e));
  }

  function setLength(next: number) {
    if (!Number.isFinite(next)) return;
    length = Math.min(MAX, Math.max(MIN, Math.round(next)));
  }

  function commitDraft() {
    if (draft !== null) {
      const parsed = Number.parseInt(draft, 10);
      setLength(Number.isFinite(parsed) ? parsed : length);
      draft = null;
    }
    persist();
  }

  function typeDraft(text: string) {
    draft = text;
    const parsed = Number.parseInt(text, 10);
    if (Number.isFinite(parsed) && parsed >= lowest && parsed <= highest) length = parsed;
  }

  function toggle(name: ClassName, on: boolean) {
    chosen[name] = on;
    persist();
  }

  async function generate() {
    if (busy) return;
    const mine = ++genSeq;
    const unchanged = lockGuard();
    error = null;
    busy = true;
    const asked = length;
    try {
      const made = await api.generatorGenerate({
        options: { length: asked, lowercase: chosen.lowercase, uppercase: chosen.uppercase, digits: chosen.digits, symbols: chosen.symbols },
        host: site === "" ? null : site,
        recordUrl: null,
      });
      if (mine !== genSeq || !unchanged()) return;
      value = made.value;
      sentLength = asked;
      effectiveLength = made.effective.length;
    } catch (e) {
      if (mine !== genSeq || !unchanged()) return;
      value = null;
      error = errorMessage(e);
    } finally {
      if (mine === genSeq) busy = false;
    }
  }

  /** What copying records: the typed website (the page URL while it is still the form's host). */
  const recordTarget = $derived(site === "" ? null : site === host && recordUrl !== null ? recordUrl : site);

  async function copy() {
    if (value === null) return;
    const current = value;
    await toasts.guard(async () => {
      await api.generatorCopy(current, recordTarget);
      announceCopied();
    });
  }

  async function loadHistory() {
    const mine = ++historySeq;
    const unchanged = lockGuard();
    try {
      const items = await api.generatorHistoryList();
      if (mine !== historySeq || !unchanged()) return;
      for (const item of items) if (!revealers.has(item.id)) revealers.set(item.id, group.create());
      history = items;
    } catch (e) {
      if (mine !== historySeq || !unchanged()) return;
      toasts.reportError(e);
    }
  }

  const TABS = ["generate", "history"] as const;
  const uid = $props.id();
  const tabId = (name: string) => `generator-tab-${uid}-${name}`;

  function onTabKey(event: KeyboardEvent, index: number) {
    const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
    if (step === 0) return;
    event.preventDefault();
    const next = TABS[(index + step + TABS.length) % TABS.length]!;
    selectTab(next);
    document.getElementById(tabId(next))?.focus();
  }

  function selectTab(next: "generate" | "history") {
    if (next === tab) return;
    tab = next;
    group.hideAll();
    if (next === "history") void loadHistory();
  }

  async function toggleReveal(id: string) {
    const r = revealers.get(id);
    if (!r) return;
    if (r.value !== null) {
      r.hide();
      return;
    }
    await toasts.guard(() => r.reveal(() => api.generatorHistoryReveal(id)));
  }

  async function copyHistory(id: string) {
    await toasts.guard(async () => {
      await api.generatorHistoryCopy(id);
      announceCopied();
    });
  }

  async function removeHistory(id: string) {
    revealers.get(id)?.hide();
    await toasts.guard(async () => {
      await api.generatorHistoryDelete(id);
      await loadHistory();
    });
  }

  const when = (item: GeneratorHistoryItem) => (item.created === null ? t("Unknown date") : formatDate(item.created, "datetime"));
</script>

<div class="generator">
  <div class="tabs" role="tablist" aria-label={t("Password Generator…")}>
    {#each TABS as name, i (name)}
      <button
        type="button"
        role="tab"
        id={tabId(name)}
        aria-selected={tab === name}
        aria-controls={`${tabId(name)}-panel`}
        tabindex={tab === name ? 0 : -1}
        onclick={() => selectTab(name)}
        onkeydown={(e) => onTabKey(e, i)}
      >
        {name === "generate" ? t("Generate") : t("Recently generated")}
      </button>
    {/each}
  </div>

  {#if tab === "generate"}
    <div class="panel" role="tabpanel" id={`${tabId("generate")}-panel`} aria-labelledby={tabId("generate")}>
      <div class="row">
        <span class="caption">{t("Length")}</span>
        <input
          type="range"
          aria-label={t("Length")}
          min={lowest}
          max={highest}
          value={shownLength}
          oninput={(e) => {
            draft = null;
            setLength(e.currentTarget.valueAsNumber);
          }}
          onchange={(e) => {
            setLength(e.currentTarget.valueAsNumber);
            persist();
          }}
        />
        <input
          type="number"
          class="number"
          aria-label={t("Length")}
          min={lowest}
          max={highest}
          value={draft ?? shownLength}
          oninput={(e) => typeDraft(e.currentTarget.value)}
          onchange={commitDraft}
          onblur={() => draft !== null && commitDraft()}
          onkeydown={(e) => e.key === "Enter" && commitDraft()}
        />
      </div>

      <div class="toggles">
        {#each CLASSES as name (name)}
          <div class="row toggle-row">
            <span class="caption">{classLabel(name)}</span>
            <Toggle checked={isOn(name)} label={classLabel(name)} disabled={locked(name)} onchange={(on) => toggle(name, on)} />
          </div>
        {/each}
      </div>

      <TextField label={t("For website")} bind:value={website} autocomplete="off" />

      {#if requirements !== ""}
        <p class="rules">{t("This website requires: %@", [requirements])}</p>
      {/if}

      <div class="actions">
        <Button variant="primary" disabled={!anyOn || busy} onclick={() => void generate()}>{t("Generate")}</Button>
      </div>

      {#if error !== null}<p class="error" role="alert">{error}</p>{/if}

      {#if value !== null}
        <code class="value" data-testid="generated-value" translate="no">{#each segments as seg, i (i)}<span data-class={seg.cls} class={seg.cls}>{seg.text}</span>{/each}</code>
        {#if effectiveLength < sentLength}
          <p class="hint" role="status">{t("Limited to %lld characters by this website", effectiveLength)}</p>
        {/if}
      {/if}

      <div class="actions">
        <Button disabled={value === null} onclick={() => void copy()}>{t("Copy")}</Button>
        {#if onuse}
          <Button variant="primary" disabled={value === null} onclick={() => value !== null && onuse(value)}>{t("Use password")}</Button>
        {/if}
      </div>
    </div>
  {:else}
    <div class="panel" role="tabpanel" id={`${tabId("history")}-panel`} aria-labelledby={tabId("history")}>
      {#if history !== null && history.length === 0}
        <p class="empty">{t("No generated passwords yet")}</p>
      {:else if history !== null}
        <ul class="history" aria-label={t("Generated passwords")}>
          {#each history as item (item.id)}
            {@const r = revealers.get(item.id)}
            <li>
              <div class="info">
                <span class="title">{item.title === "" ? item.url : item.title}</span>
                <span class="date">{when(item)}</span>
                {#if r && r.value !== null}
                  <code class="secret" translate="no">{r.value}</code>
                {:else}
                  <span class="mask" aria-hidden="true">{MASK}</span>
                {/if}
              </div>
              <div class="row-actions">
                <Button onclick={() => void toggleReveal(item.id)}>{r && r.value !== null ? t("Hide") : t("Reveal")}</Button>
                <Button onclick={() => void copyHistory(item.id)}>{t("Copy")}</Button>
                <Button variant="danger" onclick={() => void removeHistory(item.id)}>{t("Delete")}</Button>
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}
</div>

<style>
  .generator {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 320px;
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius);
    background: var(--control-border);
  }
  .tabs button {
    flex: 1;
    padding: 3px 10px;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--text);
  }
  .tabs button[aria-selected="true"] {
    background: var(--bg);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
  }
  .panel {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .caption {
    min-width: 76px;
    color: var(--text-2);
  }
  .row input[type="range"] {
    flex: 1;
  }
  .number {
    width: 64px;
    padding: 3px 6px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
    background: var(--control-bg);
    color: var(--text);
  }
  .toggle-row {
    justify-content: space-between;
  }
  .rules,
  .hint {
    margin: 0;
    color: var(--text-2);
    font-size: 12px;
  }
  .error {
    margin: 0;
    color: var(--danger, #c4342d);
  }
  .actions {
    display: flex;
    gap: 8px;
  }
  .value,
  .secret {
    display: block;
    padding: 8px 10px;
    overflow-wrap: anywhere;
    border: var(--sep);
    border-radius: var(--radius);
    background: var(--list-bg);
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 15px;
    user-select: all;
  }
  .digit {
    color: var(--accent);
  }
  .symbol {
    color: var(--danger, #c4342d);
  }
  .empty {
    margin: 0;
    color: var(--text-2);
  }
  .history {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 0;
    padding: 0;
    list-style: none;
    max-height: 320px;
    overflow: auto;
  }
  .history li {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-bottom: 8px;
    border-bottom: var(--sep);
  }
  .info {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 4px 10px;
  }
  .title {
    font-weight: 600;
  }
  .date {
    color: var(--text-2);
    font-size: 12px;
  }
  .mask {
    color: var(--text-2);
    letter-spacing: 1px;
  }
  .row-actions {
    display: flex;
    gap: 6px;
  }
</style>
