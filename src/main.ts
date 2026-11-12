import { getCurrentWindow } from "@tauri-apps/api/window";
import { mount, type Component } from "svelte";
import "./app.css";
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

// The empty shell paints first; the i18n and settings bootstrap (added by later tasks) is awaited
// after this mount, never before it.
const target = document.getElementById("app");
if (target) {
  mount(rootFor(getCurrentWindow().label), { target });
}
