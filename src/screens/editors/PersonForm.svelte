<script lang="ts">
  // The fields of a person and the Address entries it links to, in order: the first linked address
  // is the default one the browser fills with the person. A link to an entry that is gone (deleted
  // or purged) is not shown, like in the detail; it is dropped the next time the list is changed.
  // The birth date is an ISO date from the date input; the backend validates it again on save.
  import { onMount } from "svelte";
  import { api } from "../../lib/api";
  import type { EntrySummary_Serialize, PersonDto } from "../../lib/bindings";
  import { t } from "../../lib/i18n/index.svelte";
  import Button from "../../lib/ui/Button.svelte";
  import TextField from "../../lib/ui/TextField.svelte";

  let { person = $bindable(), birthDateError = null }: { person: PersonDto; birthDateError?: string | null } =
    $props();

  let addresses = $state<EntrySummary_Serialize[]>([]);
  let loaded = $state(false);

  onMount(() => {
    let live = true;
    api
      .entryList({ scope: { kind: "category", category: "addresses" }, search: "", sort: "title", order: "ascending" })
      .then((rows) => {
        if (live) addresses = rows;
      })
      .catch(() => {})
      .finally(() => {
        if (live) loaded = true;
      });
    return () => {
      live = false;
    };
  });

  const known = $derived(new Map(addresses.map((a) => [a.id, a])));
  const linked = $derived(
    person.addressIds.map((id) => known.get(id)).filter((a): a is EntrySummary_Serialize => a !== undefined),
  );
  const unlinked = $derived(addresses.filter((a) => !person.addressIds.includes(a.id)));

  function setLinks(ids: string[]) {
    person.addressIds = ids;
  }
  const linkedIds = () => linked.map((a) => a.id);

  function link(id: string) {
    setLinks([...linkedIds(), id]);
  }
  function unlink(id: string) {
    setLinks(linkedIds().filter((x) => x !== id));
  }
  function makeDefault(id: string) {
    setLinks([id, ...linkedIds().filter((x) => x !== id)]);
  }
</script>

<div class="form">
  <TextField label={t("Label")} bind:value={person.label} />
  <TextField label={t("Given Name")} bind:value={person.givenName} autocomplete="given-name" />
  <TextField label={t("Middle name")} bind:value={person.middleName} autocomplete="additional-name" />
  <TextField label={t("Family Name")} bind:value={person.familyName} autocomplete="family-name" />
  <TextField label={t("Email")} bind:value={person.email} autocomplete="email" inputmode="email" />
  <TextField label={t("Birth date")} type="date" bind:value={person.birthDate} autocomplete="bday" error={birthDateError} />

  <section class="links" aria-label={t("Linked addresses")}>
    <h3 class="caption">{t("Linked addresses")}</h3>
    {#if linked.length === 0}
      <p class="hint">{t("No linked addresses")}</p>
    {:else}
      <ul>
        {#each linked as address, index (address.id)}
          <li>
            <span class="name">
              <span class="title">{address.title}</span>
              {#if address.subtitle}<span class="sub">{address.subtitle}</span>{/if}
            </span>
            {#if index === 0}
              <span class="badge">{t("Default")}</span>
            {:else}
              <Button variant="plain" onclick={() => makeDefault(address.id)} aria-label={t("Set {0} as default", [address.title])}>
                {t("Set as default")}
              </Button>
            {/if}
            <Button variant="plain" onclick={() => unlink(address.id)} aria-label={t("Unlink {0}", [address.title])}>
              {t("Unlink")}
            </Button>
          </li>
        {/each}
      </ul>
    {/if}
    {#if unlinked.length > 0}
      <h3 class="caption">{t("Other addresses")}</h3>
      <ul>
        {#each unlinked as address (address.id)}
          <li>
            <span class="name">
              <span class="title">{address.title}</span>
              {#if address.subtitle}<span class="sub">{address.subtitle}</span>{/if}
            </span>
            <Button variant="plain" onclick={() => link(address.id)} aria-label={t("Link {0}", [address.title])}>
              {t("Link address")}
            </Button>
          </li>
        {/each}
      </ul>
    {:else if loaded && addresses.length === 0}
      <p class="hint">{t("Add an address first to link it here.")}</p>
    {/if}
  </section>
</div>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .links {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .caption {
    margin: 0;
    color: var(--text-2);
    font-size: 12px;
    font-weight: normal;
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  li {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .name {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .title,
  .sub {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub,
  .hint {
    color: var(--text-2);
    font-size: 12px;
  }
  .hint {
    margin: 0;
  }
  .badge {
    color: var(--text-2);
    font-size: 11px;
  }
</style>
