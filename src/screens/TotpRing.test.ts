import { render, screen } from "@testing-library/svelte";
import { describe, expect, it } from "vitest";
import { expectNoAxeViolations } from "../test/axe";
import TotpRing from "./TotpRing.svelte";

describe("TotpRing", () => {
  it("totp_ring_progress_and_countdown", () => {
    const { container } = render(TotpRing, { props: { remaining: 12, period: 30 } });
    const ring = screen.getByRole("img");
    expect(ring.getAttribute("aria-label")).toContain("12");
    expect(ring.textContent).toContain("12");
    const progress = container.querySelector("[data-part='progress']");
    expect(Number(progress?.getAttribute("stroke-dashoffset"))).toBeCloseTo(12 / 30, 5);
  });

  it("the_offset_follows_the_remaining_seconds", async () => {
    const { container, rerender } = render(TotpRing, { props: { remaining: 30, period: 30 } });
    const offset = () => Number(container.querySelector("[data-part='progress']")?.getAttribute("stroke-dashoffset"));
    expect(offset()).toBeCloseTo(1, 5);
    await rerender({ remaining: 0, period: 30 });
    expect(offset()).toBeCloseTo(0, 5);
  });

  it("a_zero_period_does_not_divide_by_zero", () => {
    const { container } = render(TotpRing, { props: { remaining: 0, period: 0 } });
    expect(container.querySelector("[data-part='progress']")?.getAttribute("stroke-dashoffset")).toBe("0");
  });

  it("passes_axe", async () => {
    const { container } = render(TotpRing, { props: { remaining: 12, period: 30 } });
    await expectNoAxeViolations(container);
  });
});
