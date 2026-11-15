// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(_error) = mypass_app_lib::run() {
        mypass_app_lib::startup::show_error();
        std::process::exit(1);
    }
}
