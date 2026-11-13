import { render } from "@testing-library/svelte";
import type { Component, ComponentProps } from "svelte";
import { createFakeBackend, type FakeBackend } from "./fakeBackend";

/**
 * Installs the fake backend as the IPC, renders the component with Testing Library and returns the
 * Testing Library utilities plus `fake` (call log, `emit`, `handlers`).
 */
export function renderWithBackend<C extends Component<any>>(
  component: C,
  props?: ComponentProps<C>,
  fake: FakeBackend = createFakeBackend(),
) {
  fake.install();
  const utils = render(component as Component<any>, props === undefined ? undefined : ({ props } as never));
  return { ...utils, fake };
}
