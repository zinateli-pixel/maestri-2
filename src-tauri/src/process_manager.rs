//! Gerenciador de processos: um processo por agente.
//! Responsável por iniciar/parar/enviar input/redimensionar processos reais
//! e por emitir eventos Tauri para o frontend.

use crate::context_router::{ProtocolDirective, ProtocolScanner};
use crate::models::{Agent, Status};
use crate::state::AgentBusMessage;
use crate::runtime::{adapter_for, ProcessHandle, RuntimeError};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

/// Eventos emitidos ao frontend.
pub const EVENT_AGENT_STARTED: &str = "agent_started";
pub const EVENT_AGENT_OUTPUT: &str = "agent_output";
pub const EVENT_AGENT_STOPPED: &str = "agent_stopped";
pub const EVENT_AGENT_ERROR: &str = "agent_error";
pub const EVENT_AGENT_STATUS_CHANGED: &str = "agent_status_changed";

/// Atraso (ms) antes de injetar a mensagem de descoberta de peers no agente.
/// Dá tempo para a TUI (opencode/claude/cline/kilo) terminar de iniciar e
/// ficar no prompt, evitando que o texto seja descartado durante o boot.
pub const DISCOVERY_INJECT_DELAY_MS: u64 = 2000;

/// Payload de evento genérico: `{ agentId, data }`.
#[derive(Clone, serde::Serialize)]
pub struct AgentEvent {
    #[serde(rename = "agentId")]
    pub agent_id: String,
    pub data: String,
}

/// Gerenciador de processos em execução.
pub struct ProcessManager {
    /// Processos ativos, indexados por agent id.
    pub processes: Mutex<HashMap<String, ProcessHandle>>,
    /// Workspace de cada processo em execução (agent_id -> workspace_id).
    /// Garante o isolamento: uma mensagem roteada só alcança o processo que
    /// pertence ao workspace atual, nunca o de outro workspace/instância.
    pub workspace_of: Mutex<HashMap<String, String>>,
}

impl Clone for ProcessManager {
    fn clone(&self) -> Self {
        Self {
            processes: Mutex::new(HashMap::new()),
            workspace_of: Mutex::new(HashMap::new()),
        }
    }
}

impl ProcessManager {
    pub fn new() -> Self {
        Self {
            processes: Mutex::new(HashMap::new()),
            workspace_of: Mutex::new(HashMap::new()),
        }
    }

    /// Verifica se um agente possui processo ativo PERTENCENTE ao workspace.
    pub fn is_running_in(&self, workspace_id: &str, agent_id: &str) -> bool {
        let guard = self.workspace_of.lock().expect("workspace_of mutex poisoned");
        matches!(guard.get(agent_id), Some(w) if w == workspace_id)
    }

    /// Inicia o processo de um agente e emite eventos de saída/saída.
    pub fn start(&self, app: &AppHandle, agent: &Agent) -> Result<(), RuntimeError> {
        let agent_id = agent.id.clone();

        // Rastreia o workspace ao qual este processo pertence (isolamento).
        let workspace_id = app
            .try_state::<crate::state::AppState>()
            .map(|s| {
                s.workspace
                    .lock()
                    .expect("workspace mutex poisoned")
                    .metadata
                    .id
                    .clone()
            })
            .unwrap_or_default();

        // Evita duplicação: se já existe, retorna erro.
        {
            let guard = self.processes.lock().expect("processes mutex poisoned");
            if guard.contains_key(&agent_id) {
                return Err(RuntimeError("agente já está em execução".to_string()));
            }
        }

        let app_started = app.clone();
        let app_output = app.clone();
        let app_route = app.clone();
        let app_exit = app.clone();
        let app_error = app.clone();
        let id_started = agent_id.clone();
        let id_output = agent_id.clone();
        let id_exit = agent_id.clone();
        let id_error = agent_id.clone();
        let workspace_id_output = workspace_id.clone();

        // Scanner de protocolo por processo: detecta envios iniciados pelo
        // agente (`[[MESTRO:send <alvo>]] <payload>`) sem atrasar o output.
        let scanner = Arc::new(Mutex::new(ProtocolScanner::new()));
        let scanner_for_output = Arc::clone(&scanner);

        let handle = adapter_for(agent.runtime).start(
            agent,
            Box::new(move |text| {
                let agent_id = id_output.clone();

                let (forward, directives) = {
                    let mut sc = scanner_for_output.lock().expect("scanner mutex poisoned");
                    let res = sc.feed(&text);
                    (res.forward, res.directives)
                };

                // Direções de protocolo: roteia pela topologia do workspace.
                for directive in directives {
                    match directive {
                        ProtocolDirective::Send { target, payload } => {
                            if let Some(state) = app_route.try_state::<crate::state::AppState>() {
                                let _ = state.route_context_to_peer(
                                    &app_route,
                                    &agent_id,
                                    &target,
                                    &payload,
                                );
                            }
                        }
                        ProtocolDirective::Peers => {
                            if let Some(state) = app_route.try_state::<crate::state::AppState>() {
                                state.announce_peers_to(&agent_id);
                            }
                        }
                    }
                }

                if forward.is_empty() {
                    return;
                }

                // Mantém o output no barramento interno para que
                // o orquestrador possa consultar o histórico.
                // O State é obtido através do AppHandle.
                if let Some(state) = app_output.try_state::<crate::state::AppState>() {
                    state.agent_bus.push(AgentBusMessage {
                        workspace_id: workspace_id_output.clone(),
                        agent_id: agent_id.clone(),
                        direction: "output".to_string(),
                        data: forward.clone(),
                        timestamp: crate::agent_bus::now_ms(),
                    });
                }

                // Mantém o comportamento atual do terminal.
                let _ = app_output.emit(
                    EVENT_AGENT_OUTPUT,
                    AgentEvent {
                        agent_id,
                        data: forward,
                    },
                );
            }),
            Box::new(move |status| {
                let _ = app_exit.emit(
                    EVENT_AGENT_STOPPED,
                    AgentEvent {
                        agent_id: id_exit.clone(),
                        data: status.to_string(),
                    },
                );
            }),
        )?;

        // Registra o handle.
        {
            let mut guard = self.processes.lock().expect("processes mutex poisoned");
            guard.insert(agent_id.clone(), handle);
        }
        // Registra o workspace do processo (isolamento).
        {
            let mut ws = self.workspace_of.lock().expect("workspace_of mutex poisoned");
            ws.insert(agent_id.clone(), workspace_id);
        }

        let _ = app_started.emit(
            EVENT_AGENT_STARTED,
            AgentEvent {
                agent_id: id_started,
                data: String::new(),
            },
        );

        let _ = app_error.emit(
            EVENT_AGENT_STATUS_CHANGED,
            AgentEvent {
                agent_id: id_error,
                data: Status::Running.as_str().to_string(),
            },
        );

        // Descoberta (Fase 4): anuncia ao agente, APÓS a TUI terminar de iniciar,
        // quais são seus peers conectados e como enviar contexto a eles. O atraso
        // evita que a mensagem seja descartada durante o boot da TUI.
        {
            let app_for_banner = app.clone();
            let id_for_banner = agent_id.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(
                    DISCOVERY_INJECT_DELAY_MS,
                ));
                if let Some(state) = app_for_banner.try_state::<crate::state::AppState>() {
                    state.announce_peers_to(&id_for_banner);
                }
            });
        }

        Ok(())
    }

    /// Envia input ao processo de um agente.
    pub fn send_input(&self, agent_id: &str, input: &str) -> Result<(), RuntimeError> {
        let mut guard = self.processes.lock().expect("processes mutex poisoned");
        let handle = guard
            .get_mut(agent_id)
            .ok_or_else(|| RuntimeError("agente não está em execução".to_string()))?;
        handle.write_input(input.as_bytes())
    }

    /// Envia input a um agente SOMENTE se o processo pertencer ao workspace
    /// informado. É a fronteira de isolamento da comunicação entre agentes.
    pub fn send_input_in(
        &self,
        workspace_id: &str,
        agent_id: &str,
        input: &str,
    ) -> Result<(), RuntimeError> {
        self.assert_workspace(workspace_id, agent_id)?;
        self.send_input(agent_id, input)
    }

    /// Redimensiona o PTY de um agente SOMENTE se o processo pertencer ao
    /// workspace informado.
    pub fn resize_in(
        &self,
        workspace_id: &str,
        agent_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), RuntimeError> {
        self.assert_workspace(workspace_id, agent_id)?;
        self.resize(agent_id, cols, rows)
    }

    /// Valida que `agent_id` está associado a `workspace_id` no mapa de
    /// isolamento. Usado por todas as operações escopadas por workspace.
    fn assert_workspace(&self, workspace_id: &str, agent_id: &str) -> Result<(), RuntimeError> {
        let ws = self.workspace_of.lock().expect("workspace_of mutex poisoned");
        match ws.get(agent_id) {
            Some(w) if w == workspace_id => Ok(()),
            _ => Err(RuntimeError(
                "agente não pertence ao workspace atual".to_string(),
            )),
        }
    }

    /// Redimensiona o PTY de um agente.
    pub fn resize(&self, agent_id: &str, cols: u16, rows: u16) -> Result<(), RuntimeError> {
        let mut guard = self.processes.lock().expect("processes mutex poisoned");
        let handle = guard
            .get_mut(agent_id)
            .ok_or_else(|| RuntimeError("agente não está em execução".to_string()))?;
        handle.resize(cols, rows)
    }

    /// Para o processo de um agente (SIGKILL) e remove do mapa.
    pub fn stop(&self, app: &AppHandle, agent_id: &str) -> Result<(), RuntimeError> {
        eprintln!("[PROCESS_MGR] stop begin agent_id={}", agent_id);
        let mut handle = {
            let mut guard = self.processes.lock().expect("processes mutex poisoned");
            guard
                .remove(agent_id)
                .ok_or_else(|| RuntimeError("agente não está em execução".to_string()))?
        };
        eprintln!("[PROCESS_MGR] stop: handle removed from map, calling kill()...");

        // Remove o vínculo workspace -> agente ANTES do kill, para não deixar
        // entrada órfã em workspace_of caso o kill falhe.
        {
            let mut ws = self.workspace_of.lock().expect("workspace_of mutex poisoned");
            ws.remove(agent_id);
        }

        handle.kill()?;
        eprintln!("[PROCESS_MGR] stop: kill() returned");

        let _ = app.emit(
            EVENT_AGENT_STOPPED,
            AgentEvent {
                agent_id: agent_id.to_string(),
                data: "stopped".to_string(),
            },
        );
        eprintln!("[PROCESS_MGR] stop complete");
        Ok(())
    }

    /// Para todos os processos (usado no encerramento do app).
    pub fn stop_all(&self) {
        let mut guard = self.processes.lock().expect("processes mutex poisoned");
        for (_, handle) in guard.iter_mut() {
            let _ = handle.kill();
        }
        guard.clear();
        self.workspace_of
            .lock()
            .expect("workspace_of mutex poisoned")
            .clear();
    }
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_running_in_is_scoped_by_workspace() {
        let pm = ProcessManager::new();
        pm.workspace_of
            .lock()
            .unwrap()
            .insert("agent-x".to_string(), "ws-A".to_string());

        assert!(pm.is_running_in("ws-A", "agent-x"));
        assert!(!pm.is_running_in("ws-B", "agent-x"));
        assert!(!pm.is_running_in("ws-A", "agent-y"));
    }

    #[test]
    fn send_input_in_rejects_process_from_other_workspace() {
        let pm = ProcessManager::new();

        // Sem vínculo: rejeita (não pertence ao workspace).
        assert!(pm.send_input_in("ws-A", "agent-x", "hi").is_err());

        // Vínculo em OUTRO workspace: rejeita.
        pm.workspace_of
            .lock()
            .unwrap()
            .insert("agent-x".to_string(), "ws-B".to_string());
        assert!(pm.send_input_in("ws-A", "agent-x", "hi").is_err());
    }

    #[test]
    fn resize_in_rejects_process_from_other_workspace() {
        let pm = ProcessManager::new();

        // Sem vínculo: rejeita.
        assert!(pm.resize_in("ws-A", "agent-x", 80, 24).is_err());

        // Vínculo em OUTRO workspace: rejeita.
        pm.workspace_of
            .lock()
            .unwrap()
            .insert("agent-x".to_string(), "ws-B".to_string());
        assert!(pm.resize_in("ws-A", "agent-x", 80, 24).is_err());
    }
}