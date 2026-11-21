<script lang="ts">
  import type { AddressDto, FieldRef } from "../lib/bindings";
  import { t } from "../lib/i18n/index.svelte";
  import DetailField from "./DetailField.svelte";

  let { address, entryId, onlargetype }: {
    address: AddressDto;
    entryId: string;
    onlargetype: (field: FieldRef) => void;
  } = $props();

  const countryKey = $derived(address.countryName.trim() ? "countryName" : "countryCode");
  const rows = $derived(
    [
      { key: "label", label: t("Label"), value: address.label },
      { key: "fullName", label: t("Full Name"), value: address.fullName },
      { key: "givenName", label: t("Given Name"), value: address.givenName },
      { key: "familyName", label: t("Family Name"), value: address.familyName },
      { key: "organization", label: t("Organization"), value: address.organization },
      { key: "addressLine1", label: t("Address Line 1"), value: address.addressLine1 },
      { key: "addressLine2", label: t("Address Line 2"), value: address.addressLine2 },
      { key: "city", label: t("City"), value: address.city },
      { key: "region", label: t("Region"), value: address.region },
      { key: "postalCode", label: t("Postal Code"), value: address.postalCode },
      { key: countryKey, label: t("Country"), value: address[countryKey] },
      { key: "phone", label: t("Phone"), value: address.phone },
      { key: "email", label: t("Email"), value: address.email },
    ].filter((r) => r.value.trim() !== ""),
  );
</script>

{#each rows as row (row.key)}
  <DetailField label={row.label} {entryId} field={{ kind: "address", name: row.key }} text={row.value} {onlargetype} />
{/each}
