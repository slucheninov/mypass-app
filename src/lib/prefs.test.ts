import { describe, expect, it } from "vitest";
import { readPref, writePref } from "./prefs";

describe("prefs", () => {
  it("only_layout_keys_are_stored", () => {
    writePref("layout.widths", "[1,2]");
    expect(readPref("layout.widths")).toBe("[1,2]");
    writePref("vault.title", "secret");
    expect(localStorage.getItem("vault.title")).toBeNull();
    localStorage.setItem("vault.other", "x");
    expect(readPref("vault.other")).toBeNull();
    localStorage.clear();
  });
});
