<script lang="ts">
  // The 14 fields of an address: the label (Home, Work, Other) and 13 text fields. The label is
  // stored in English whatever the UI language; the backend folds anything else into Other.
  import type { AddressDto } from "../../lib/bindings";
  import { t } from "../../lib/i18n/index.svelte";
  import Select from "../../lib/ui/Select.svelte";
  import TextField from "../../lib/ui/TextField.svelte";

  let { address = $bindable() }: { address: AddressDto } = $props();

  const labels = $derived([
    { value: "Home", label: t("Home") },
    { value: "Work", label: t("Work") },
    { value: "Other", label: t("Other") },
  ]);
</script>

<div class="form">
  <div class="label-row">
    <span class="caption">{t("Label")}</span>
    <Select value={address.label} options={labels} label={t("Label")} onchange={(v) => (address.label = v)} />
  </div>
  <TextField label={t("Full Name")} bind:value={address.fullName} autocomplete="name" />
  <TextField label={t("Given Name")} bind:value={address.givenName} autocomplete="given-name" />
  <TextField label={t("Family Name")} bind:value={address.familyName} autocomplete="family-name" />
  <TextField label={t("Organization")} bind:value={address.organization} autocomplete="organization" />
  <TextField label={t("Address Line 1")} bind:value={address.addressLine1} autocomplete="address-line1" />
  <TextField label={t("Address Line 2")} bind:value={address.addressLine2} autocomplete="address-line2" />
  <TextField label={t("City")} bind:value={address.city} autocomplete="address-level2" />
  <TextField label={t("Region")} bind:value={address.region} autocomplete="address-level1" />
  <TextField label={t("Postal Code")} bind:value={address.postalCode} autocomplete="postal-code" />
  <TextField label={t("Country Code")} bind:value={address.countryCode} autocomplete="country" />
  <TextField label={t("Country")} bind:value={address.countryName} autocomplete="country-name" />
  <TextField label={t("Phone")} bind:value={address.phone} autocomplete="tel" inputmode="tel" />
  <TextField label={t("Email")} bind:value={address.email} autocomplete="email" inputmode="email" />
</div>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .label-row {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .caption {
    color: var(--text-2);
    font-size: 12px;
  }
</style>
