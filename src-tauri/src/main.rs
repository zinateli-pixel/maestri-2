// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("agent-bridge") {
        if let Err(error) = tauri_app_lib::agent_bridge::run_client(&args[2..]) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    tauri_app_lib::run()
}
