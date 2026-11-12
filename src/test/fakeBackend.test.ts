import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { invoke } from "@tauri-apps/api/core";
import { describe, expect, it } from "vitest";
import { createFakeBackend } from "./fakeBackend";
import { renderWithBackend } from "./render";
import MainApp from "../windows/MainApp.svelte";
import { commands } from "../lib/bindings";

describe("fake backend", () => {
  it("fake_backend_covers_every_binding_command", () => {
    // Parse the generated file instead of importing it, so a command the fake forgot cannot hide
    // behind the same object the fake was built from.
    const text = readFileSync(resolve(process.cwd(), "src/lib/bindings.ts"), "utf8");
    const block = /export const commands = \{([\s\S]*?)\n\};/.exec(text)?.[1] ?? "";
    const names = [...block.matchAll(/^\t([a-zA-Z0-9]+): /gm)].map((m) => m[1]).sort();
    expect(names.length).toBeGreaterThan(0);
    expect(Object.keys(createFakeBackend().handlers).sort()).toEqual(names);
    expect(Object.keys(commands).sort()).toEqual(names);
    // The fake derives the wire name with `snake()`; it must equal the string the binding invokes.
    const wire = new Map(
      block
        .split(/^\t(?=[a-zA-Z0-9]+: )/m)
        .map((chunk) => /^([a-zA-Z0-9]+): [\s\S]*?__TAURI_INVOKE(?:<.*?>)?\("([a-z0-9_]+)"/.exec(chunk))
        .filter((m) => m !== null)
        .map((m) => [m[1], m[2]] as const),
    );
    expect(wire.size).toBe(names.length);
    const snake = (n: string) => n.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`);
    for (const [name, wireName] of wire) expect(snake(name ?? ""), name).toBe(wireName);
  });

  it("fake_app_info_reports_device_unlock_flag_and_os", async () => {
    const fake = createFakeBackend({ os: "windows", deviceUnlockAvailable: true });
    renderWithBackend(MainApp, {}, fake);
    expect(await commands.appInfo()).toEqual({
      status: "ok",
      data: { version: expect.any(String), os: "windows", deviceUnlockAvailable: true },
    });
    expect(fake.calls.map((c) => c.command)).toEqual(["appInfo"]);
  });

  it("platform_keyboard_layout_is_null_unless_configured", async () => {
    renderWithBackend(MainApp, {}, createFakeBackend());
    expect(await commands.platformKeyboardLayout()).toEqual({ status: "ok", data: null });
    renderWithBackend(MainApp, {}, createFakeBackend({ keyboardLayout: "UK" }));
    expect(await commands.platformKeyboardLayout()).toEqual({ status: "ok", data: "UK" });
  });

  it("an unimplemented command fails loudly", async () => {
    renderWithBackend(MainApp, {}, createFakeBackend());
    await expect(commands.vaultFindFiles()).rejects.toThrow("NotImplementedInFake(vaultFindFiles)");
  });

  it("an unknown command or an unhandled plugin call throws and names itself", async () => {
    renderWithBackend(MainApp, {}, createFakeBackend());
    await expect(invoke("no_such_command")).rejects.toThrow("NotImplementedInFake(unknown command no_such_command)");
    await expect(invoke("plugin:opener|open_url")).rejects.toThrow(
      "NotImplementedInFake(plugin:opener|open_url)",
    );
  });

  it("a plugin call works when the test opts in", async () => {
    const seen: unknown[] = [];
    const fake = createFakeBackend({
      plugins: { "plugin:opener|open_url": (p) => (seen.push(p), null) },
    });
    renderWithBackend(MainApp, {}, fake);
    await invoke("plugin:opener|open_url", { url: "u" });
    expect(seen).toEqual([{ url: "u" }]);
  });

  it("emit reaches listeners registered through the generated events", async () => {
    const fake = createFakeBackend();
    renderWithBackend(MainApp, {}, fake);
    const { events } = await import("../lib/bindings");
    const seen: unknown[] = [];
    const un = await events.vaultChanged.listen((e) => seen.push(e.payload));
    await fake.emit("vaultChanged", null);
    un();
    expect(seen).toEqual([null]);
  });
});
