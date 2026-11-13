import { render, screen } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { toasts } from "../stores/toasts.svelte";
import ToastHost from "./ToastHost.svelte";

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  toasts.clear();
  vi.useRealTimers();
});

describe("toast host", () => {
  it("toast_auto_dismisses_success_after_4s_but_not_errors", () => {
    render(ToastHost);
    toasts.push("success", "Done");
    toasts.push("info", "FYI");
    toasts.push("error", "Broke", { sticky: true });
    vi.advanceTimersByTime(3999);
    expect(toasts.items).toHaveLength(3);
    vi.advanceTimersByTime(1);
    expect(toasts.items.map((x) => x.text)).toEqual(["Broke"]);
    vi.advanceTimersByTime(60_000);
    expect(toasts.items).toHaveLength(1);
  });

  it("toasts_have_status_and_alert_roles", async () => {
    render(ToastHost);
    toasts.push("success", "Done");
    toasts.push("info", "FYI");
    toasts.push("error", "Broke", { sticky: true });
    expect(await screen.findAllByRole("status")).toHaveLength(2);
    expect((await screen.findByRole("alert")).textContent).toContain("Broke");
  });

  it("the_host_is_one_labelled_region", () => {
    render(ToastHost);
    expect(screen.getAllByRole("region", { name: "Notifications" })).toHaveLength(1);
  });
});
