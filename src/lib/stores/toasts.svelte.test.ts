import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { expectNoAxeViolations } from "../../test/axe";
import ToastHost from "../ui/ToastHost.svelte";
import { ApiFailure } from "../api";
import { __resetCatalogsForTest, setLanguage, t } from "../i18n/index.svelte";
import { toasts } from "./toasts.svelte";

beforeEach(() => {
  __resetCatalogsForTest();
  setLanguage("en");
  vi.useFakeTimers();
});
afterEach(() => {
  toasts.clear();
  vi.useRealTimers();
});

describe("toasts store", () => {
  it("push_adds_a_toast_and_dismiss_removes_it", () => {
    const id = toasts.push("info", "Hello");
    expect(toasts.items).toHaveLength(1);
    expect(toasts.items[0]).toMatchObject({ id, kind: "info", text: "Hello", sticky: false });
    toasts.dismiss(id);
    expect(toasts.items).toHaveLength(0);
  });

  it("non_sticky_toast_expires_and_sticky_stays", () => {
    toasts.push("success", "Saved");
    toasts.push("error", "Failed", { sticky: true });
    vi.advanceTimersByTime(10_000);
    expect(toasts.items.map((x) => x.text)).toEqual(["Failed"]);
  });

  it("quitting_error_toast_is_not_sticky_and_has_no_detail", () => {
    toasts.reportError(new ApiFailure("quitting", "internal wording"));
    toasts.reportError(new ApiFailure("quitting"));
    expect(toasts.items).toHaveLength(1);
    expect(toasts.items[0]).toMatchObject({ kind: "error", text: t("MyPass is quitting"), sticky: false });
    expect(toasts.items[0]?.detail).toBeUndefined();
  });

  it("report_error_localises_and_keeps_detail_only_for_io", () => {
    toasts.reportError(new ApiFailure("wrongPassword", "secret-ish"));
    toasts.reportError(new ApiFailure("io", "disk full"));
    expect(toasts.items[0]).toMatchObject({ text: t("Wrong master password") });
    expect(toasts.items[0]?.detail).toBeUndefined();
    expect(toasts.items[1]).toMatchObject({ text: t("Something went wrong"), detail: "disk full", sticky: true });
  });

  it("pause_stops_expiry_and_resume_restarts_it", () => {
    const id = toasts.push("info", "Hover me");
    vi.advanceTimersByTime(3000);
    toasts.pause(id);
    vi.advanceTimersByTime(60_000);
    expect(toasts.items).toHaveLength(1);
    toasts.resume(id);
    vi.advanceTimersByTime(3999);
    expect(toasts.items).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(toasts.items).toHaveLength(0);
  });

  it("cancelled_is_swallowed", () => {
    toasts.reportError(new ApiFailure("cancelled"));
    expect(toasts.items).toHaveLength(0);
  });
});

describe("ToastHost", () => {
  it("host_renders_text_detail_and_dismisses", async () => {
    vi.useRealTimers();
    const user = userEvent.setup();
    toasts.push("error", "Saving failed", { sticky: true, detail: "disk full" });
    const { container } = render(ToastHost);
    expect(screen.getByRole("region", { name: t("Notifications") })).toBeTruthy();
    expect(screen.getByText("Saving failed")).toBeTruthy();
    expect(screen.getByText(t("Details"))).toBeTruthy();
    expect(screen.getByText("disk full")).toBeTruthy();
    await expectNoAxeViolations(container);
    await user.click(screen.getByRole("button", { name: t("Close") }));
    expect(screen.queryByText("Saving failed")).toBeNull();
  });
});
