<script lang="ts">
  import Avatar from "../lib/ui/Avatar.svelte";
  import Button from "../lib/ui/Button.svelte";
  import FieldRow from "../lib/ui/FieldRow.svelte";
  import IconButton from "../lib/ui/IconButton.svelte";
  import Segmented from "../lib/ui/Segmented.svelte";
  import Select from "../lib/ui/Select.svelte";
  import Stepper from "../lib/ui/Stepper.svelte";
  import Toggle from "../lib/ui/Toggle.svelte";
  import Menu from "../lib/ui/Menu.svelte";
  import Modal from "../lib/ui/Modal.svelte";
  import SplitPane from "../lib/ui/SplitPane.svelte";
  import WindowControls from "../lib/ui/WindowControls.svelte";
  const opts = [{ value: "a", label: "Alpha" }, { value: "b", label: "Beta" }];
</script>

<Button onclick={() => {}}>Plain</Button>
<IconButton icon="plus" label="Add" onclick={() => {}} />
<Toggle checked={true} label="Enabled" onchange={() => {}} />
<Stepper value={30} min={10} max={120} step={5} label="Seconds" unit="s" onchange={() => {}} />
<Select value="a" options={opts} label="Letter" onchange={() => {}} />
<Segmented value="a" options={opts} label="Mode" onchange={() => {}} />
<FieldRow label="Username">
  <span>alice</span>
  {#snippet actions()}<button>Copy</button>{/snippet}
</FieldRow>
<Avatar text="GitHub" />
<Menu items={[{ id: "x", label: "Item" }]} onselect={() => {}} label="Menu">
  {#snippet trigger()}<span>Open</span>{/snippet}
</Menu>
<Modal open={true} title="Modal title" onclose={() => {}}><p>Body</p></Modal>
<SplitPane widths={[200, 300]} min={[160, 240, 320]} onresize={() => {}}>
  {#snippet first()}<div>one</div>{/snippet}
  {#snippet second()}<div>two</div>{/snippet}
  {#snippet third()}<div>three</div>{/snippet}
</SplitPane>
<WindowControls os="linux" />
