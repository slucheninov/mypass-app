// Prevents an extra console window on Windows in release builds.
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

fn main() {
    #[cfg(windows)]
    if !mypass_app_lib::windows_boot::preflight() {
        std::process::exit(0);
    }
    if let Err(_error) = mypass_app_lib::run() {
        mypass_app_lib::startup::show_error();
        std::process::exit(1);
    }
}
