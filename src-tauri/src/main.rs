// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(error) = mypass_app_lib::run() {
        eprintln!("MyPass failed to start: {error}");
        std::process::exit(1);
    }
}
