<script lang="ts">
  // One user-defined field. A protected field arrives without its value and is revealed on
  // request into the entry's reveal group; the label is user content, not UI text.
  import { untrack } from "svelte";
  import type { CustomFieldDto_Serialize, FieldRef } from "../lib/bindings";
  import type { RevealGroup } from "../lib/secret.svelte";
  import DetailField from "./DetailField.svelte";

  let {
    entryId,
    field,
    group,
    onlargetype,
  }: {
    entryId: string;
    field: CustomFieldDto_Serialize;
    group: RevealGroup;
    onlargetype: (field: FieldRef) => void;
  } = $props();

  const secret = untrack(() => group.create());
</script>

<DetailField
  label={field.key}
  {entryId}
  field={{ kind: "custom", key: field.key }}
  secret={field.protected ? secret : null}
  text={field.value ?? ""}
  {onlargetype}
/>
