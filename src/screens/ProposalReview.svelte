<script lang="ts">
  // The review of a registration form proposal, before its login opens in the editor: every extra
  // value of the form with a checkbox (identity and address values ticked, a field the page
  // labelled itself unticked: see `keptByDefault`), and, when the form held an address the
  // vault has no Address entry for, "Also create an Address entry" (off by default). Nothing is
  // saved here: Continue reports the choice, the handler builds the editor model from it.
  import type { BrowserProposal_Serialize } from "../lib/bindings";
  import { t } from "../lib/i18n/index.svelte";
  import { keptByDefault, proposalFieldLabel } from "../lib/proposal-fields";
  import Button from "../lib/ui/Button.svelte";
  import Modal from "../lib/ui/Modal.svelte";

  let {
    proposal,
    onconfirm,
    oncancel,
  }: {
    proposal: BrowserProposal_Serialize;
    onconfirm: (keep: boolean[], withAddress: boolean) => void;
    oncancel: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  let keep = $state(proposal.extraFields.map(keptByDefault));
  let withAddress = $state(false);
</script>

<Modal open={true} title={t("Review form fields")} subtitle={proposal.model.title} onclose={oncancel} focusDialog={true}>
  <p class="intro">{t("Choose the form fields to keep with the new login.")}</p>
  {#if proposal.extraFields.length > 0}
    <ul class="fields" data-testid="proposal-fields">
      {#each proposal.extraFields as field, i (i)}
        <li>
          <label>
            <input type="checkbox" bind:checked={keep[i]} />
            <span class="name">{proposalFieldLabel(field)}</span>
            <span class="value">{field.value}</span>
          </label>
        </li>
      {/each}
    </ul>
  {/if}
  {#if proposal.address}
    <label class="address">
      <input type="checkbox" bind:checked={withAddress} />
      <span>{t("Also create an Address entry")}</span>
      <span class="value">{proposal.address.title}</span>
    </label>
  {/if}
  {#snippet footer()}
    <Button onclick={oncancel}>{t("Cancel")}</Button>
    <Button variant="primary" onclick={() => onconfirm([...keep], withAddress)}>{t("Continue")}</Button>
  {/snippet}
</Modal>

<style>
  .intro {
    margin: 0 0 10px;
    color: var(--text-2);
  }
  .fields {
    margin: 0;
    padding: 0;
    list-style: none;
    max-height: 320px;
    overflow-y: auto;
  }
  label {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) minmax(0, 1.4fr);
    gap: 8px;
    align-items: baseline;
    padding: 4px 0;
  }
  .address {
    margin-top: 10px;
    padding-top: 10px;
    border-top: var(--sep);
  }
  .name {
    font-weight: 500;
  }
  .value {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-2);
  }
</style>
