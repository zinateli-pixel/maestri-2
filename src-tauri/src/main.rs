// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bridge_args = match args.get(1).map(String::as_str) {
        Some("agent-bridge") => Some(args[2..].to_vec()),
        // Compatibilidade mínima com a skill global `maestri`. Não cria outro
        // protocolo: apenas traduz os comandos para o agent-bridge existente.
        Some("list") => Some(vec!["peers".to_string()]),
        Some("ask") | Some("send") => Some(args[1..].to_vec()),
        _ => None,
    };
    if let Some(bridge_args) = bridge_args {
        if let Err(error) = tauri_app_lib::agent_bridge::run_client(&bridge_args) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    tauri_app_lib::run()
}
