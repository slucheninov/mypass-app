import axe from "axe-core";
import { expect } from "vitest";

/** Runs axe on the element; jsdom cannot compute colours or landmarks, so those rules are off. */
export async function expectNoAxeViolations(el: Element): Promise<void> {
  const result = await axe.run(el, {
    rules: { "color-contrast": { enabled: false }, region: { enabled: false } },
  });
  expect(result.violations.map((v) => `${v.id}: ${v.help}`)).toEqual([]);
}
