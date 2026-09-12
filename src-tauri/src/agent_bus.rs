use crate::models::{Agent, WorkspaceState};
use crate::state::AppState;
use serde::Serialize;
use tauri::State;

const HISTORY_LIMIT: usize = 200;

#[derive(Debug, Clone, Serialize)]
pub struct AgentMessage {
    pub agent_id: String,
    pub direction: String,
    pub data: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentSnapshot {
    pub agent: Agent,
    pub running: bool,
    pub history: Vec<AgentMessage>,
}

pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[tauri::command]
pub fn bus_list_agents(state: State<'_, AppState>) -> Vec<Agent> {
    let guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");

    guard.agents.clone()
}

#[tauri::command]
pub fn bus_get_workspace(state: State<'_, AppState>) -> WorkspaceState {
    let guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");

    guard.clone()
}

#[tauri::command]
pub fn bus_get_agent(
    state: State<'_, AppState>,
    id: String,
) -> Result<AgentSnapshot, String> {
    let (workspace_id, agent) = {
        let guard = state
            .workspace
            .lock()
            .map_err(|_| "workspace mutex poisoned".to_string())?;

        let ws_id = guard.metadata.id.clone();
        let agent = guard
            .agents
            .iter()
            .find(|a| a.id == id)
            .cloned()
            .ok_or_else(|| "agente não encontrado".to_string())?;
        (ws_id, agent)
    };

    // Escopado ao workspace atual: não vaza running/histórico de outro workspace.
    let running = state.processes.is_running_in(&workspace_id, &id);

    // Histórico real de mensagens do agente no barramento interno
    // (output do PTY, contexto roteado in/out, etc.).
    let history = state
        .agent_bus
        .get_agent_messages(&workspace_id, &id)
        .into_iter()
        .map(|m| AgentMessage {
            agent_id: m.agent_id,
            direction: m.direction,
            data: m.data,
            timestamp: m.timestamp,
        })
        .collect();

    Ok(AgentSnapshot {
        agent,
        running,
        history,
    })
}

#[tauri::command]
pub fn bus_send_message(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    message: String,
) -> Result<(), String> {
    if message.trim().is_empty() {
        return Err("mensagem vazia".to_string());
    }

    let workspace_id = {
        let guard = state
            .workspace
            .lock()
            .map_err(|_| "workspace mutex poisoned".to_string())?;
        guard.metadata.id.clone()
    };

    if !state.processes.is_running_in(&workspace_id, &id) {
        return Err("agente não está em execução".to_string());
    }

    state
        .processes
        .send_input_in(&workspace_id, &id, &(message + "\n"))
        .map_err(|e| e.to_string())?;

    let _ = app;

    Ok(())
}

#[tauri::command]
pub fn bus_agent_running(
    state: State<'_, AppState>,
    id: String,
) -> bool {
    let workspace_id = {
        let guard = state
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        guard.metadata.id.clone()
    };
    state.processes.is_running_in(&workspace_id, &id)
}
