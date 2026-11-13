import { getCurrentWindow } from "@tauri-apps/api/window";
import { mount, type Component } from "svelte";
import "./app.css";
import { setLanguage } from "./lib/i18n/index.svelte";
import { loadPlatform } from "./lib/platform.svelte";
import { startStores } from "./lib/stores";
import MainApp from "./windows/MainApp.svelte";
import SettingsApp from "./windows/SettingsApp.svelte";
import TrayPanel from "./windows/TrayPanel.svelte";

// One bundle serves three webview windows; the window label picks the root component. Anything
// unknown falls back to the main window.
function rootFor(label: string): Component {
  switch (label) {
    case "tray-panel":
      return TrayPanel;
    case "settings":
      return SettingsApp;
    default:
      return MainApp;
  }
}

// The language starts from the system setting; the persisted setting is applied after mount by
// `settings.start()` inside `startStores`, which never delays the first paint.
setLanguage("system");

const target = document.getElementById("app");
if (target) {
  mount(rootFor(getCurrentWindow().label), { target });
  // Bootstrap after the first paint: a slow or failing backend never blocks the window. A failure
  // leaves the stores at their safe defaults.
  void loadPlatform();
  const label = getCurrentWindow().label;
  // The sticky "Saving failed" toast is the main window's alone (unknown labels fall back to main).
  void startStores({
    entries: label !== "settings" && label !== "tray-panel",
    saveToast: label !== "settings" && label !== "tray-panel",
  }).catch(() => {});
}
