<script lang="ts">
  // The fields of a person and its linked addresses. The addresses are links to those entries in
  // the person's order (the first is the default); the backend leaves out links to entries that
  // are gone, and no address value travels with the person.
  import type { FieldRef, LinkedAddressDto, PersonDto } from "../lib/bindings";
  import { t } from "../lib/i18n/index.svelte";
  import { navigation } from "../lib/stores/navigation.svelte";
  import FieldRow from "../lib/ui/FieldRow.svelte";
  import DetailField from "./DetailField.svelte";

  let { person, addresses, entryId, onlargetype }: {
    person: PersonDto;
    addresses: LinkedAddressDto[];
    entryId: string;
    onlargetype: (field: FieldRef) => void;
  } = $props();

  const rows = $derived(
    [
      { key: "label", label: t("Label"), value: person.label },
      { key: "givenName", label: t("Given Name"), value: person.givenName },
      { key: "middleName", label: t("Middle name"), value: person.middleName },
      { key: "familyName", label: t("Family Name"), value: person.familyName },
      { key: "email", label: t("Email"), value: person.email },
      { key: "birthDate", label: t("Birth date"), value: person.birthDate },
    ].filter((r) => r.value.trim() !== ""),
  );

  function open(id: string) {
    navigation.setScope({ kind: "category", category: "addresses" });
    navigation.select(id);
  }
</script>

{#each rows as row (row.key)}
  <DetailField label={row.label} {entryId} field={{ kind: "person", name: row.key }} text={row.value} {onlargetype} />
{/each}
{#if addresses.length > 0}
  <FieldRow label={t("Linked addresses")}>
    <div role="group" aria-label={t("Linked addresses")}>
      <ul class="links">
        {#each addresses as address, index (address.id)}
          <li>
            <button type="button" class="link" onclick={() => open(address.id)}>{address.title}</button>
            {#if address.subtitle}<span class="sub">{address.subtitle}</span>{/if}
            {#if index === 0 && addresses.length > 1}<span class="badge">{t("Default")}</span>{/if}
          </li>
        {/each}
      </ul>
    </div>
  </FieldRow>
{/if}

<style>
  .links {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  li {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .link {
    padding: 0;
    border: none;
    background: none;
    color: var(--accent);
    font: inherit;
    cursor: pointer;
    text-align: left;
  }
  .link:hover {
    text-decoration: underline;
  }
  .sub,
  .badge {
    color: var(--text-2);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
