mod agent_bus;
pub mod agent_bridge;
mod app_agent;
mod chain;
mod commands;
mod context_router;
mod events;
mod mcp;
mod memory;
mod models;
mod persistence;
mod process_manager;
mod runtime;
mod skills;
mod state;
mod web_agent;
mod workflow;

use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("falha ao resolver app_data_dir");
            app.manage(AppState::new(app_data_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_workspace_state,
            commands::get_agents,
            commands::create_agent,
            commands::update_agent,
            commands::delete_agent,
            commands::update_agent_position,
            commands::update_agent_geometry,
            commands::duplicate_agent,
            commands::restart_agent,
            commands::refresh_agent,
            commands::detect_runtimes,
            commands::export_workspace,
            commands::import_workspace,
            commands::restore_workspace,
            commands::backup_workspace,
            commands::list_backups,
            // Workflow commands
            commands::list_workflows,
            commands::get_workflow,
            commands::create_workflow,
            commands::update_workflow,
            commands::delete_workflow,
            commands::list_executions,
            commands::get_execution,
            commands::start_workflow_execution,
            commands::cancel_workflow_execution,
            commands::delete_execution,
            commands::add_edge,
            commands::remove_edge,
            commands::start_agent,
            commands::stop_agent,
            commands::send_agent_input,
            commands::resize_agent,
            commands::ping,

            // Agent Bus
            agent_bus::bus_list_agents,
            agent_bus::bus_get_workspace,
            agent_bus::bus_get_agent,
            agent_bus::bus_send_message,
            agent_bus::bus_agent_running,
            // Context routing (Fase 3/4)
            context_router::route_context,
            context_router::route_context_to,
            context_router::route_context_ask,
            // Workflow chain (Fase 6/7)
            chain::run_canvas_chain,
            chain::list_workflow_events,
            // Skills (Fase 9)
            skills::list_skills,
            skills::skills_of_agent,
            skills::create_skill,
            skills::associate_skill,
            skills::delete_skill,
            // MCP Manager (Fase 10)
            mcp::list_mcp_servers,
            mcp::mcps_of_agent,
            mcp::mcps_of_skill,
            mcp::create_mcp_server,
            mcp::associate_mcp_to_agent,
            mcp::associate_mcp_to_skill,
            mcp::delete_mcp_server,
            // Memory (Fase 11)
            memory::list_memory,
            memory::memory_by_agent,
            memory::memory_shared,
            memory::create_memory,
            memory::remove_memory,
            // Workspace management
            commands::list_workspaces,
            commands::create_workspace,
            commands::load_workspace,
            commands::rename_workspace,
            commands::delete_workspace,
            commands::duplicate_workspace,
            commands::update_workspace_settings,
            commands::update_viewport,
            commands::get_workspace_list,
            // Projects (Fase 12)
            commands::list_projects,
            commands::create_project,
            commands::rename_project,
            commands::delete_project,
            commands::add_workspace_to_project,
            commands::remove_workspace_from_project
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Ao fechar o app, encerra todos os processos filhos (sem órfãos).
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let state = app.state::<AppState>();
                state.processes.stop_all();
                state.web_sessions.stop_all();
                state.app_sessions.stop_all();
            }
        });
}
