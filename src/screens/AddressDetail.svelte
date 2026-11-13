<script lang="ts">
  // The non-empty fields of an address. There is no backend copy for these fields, so the rows
  // are plain selectable text.
  import type { AddressDto } from "../lib/bindings";
  import { t } from "../lib/i18n/index.svelte";
  import FieldRow from "../lib/ui/FieldRow.svelte";

  let { address }: { address: AddressDto } = $props();

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
      { key: "country", label: t("Country"), value: address.countryName || address.countryCode },
      { key: "phone", label: t("Phone"), value: address.phone },
      { key: "email", label: t("Email"), value: address.email },
    ].filter((r) => r.value.trim() !== ""),
  );
</script>

{#each rows as row (row.key)}
  <div role="group" aria-label={row.label}>
    <FieldRow label={row.label}>{row.value}</FieldRow>
  </div>
{/each}
