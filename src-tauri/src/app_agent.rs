//! Fase 15 — Agentes de Aplicativo.
//!
//! `AgentKind::App` representa um agente que opera um aplicativo (desktop/mobile),
//! separado do meio CLI/PTY e do meio web. O controle real do aplicativo é a
//! Fase 16 (Computer Control); aqui estabelecemos a FRONTEIRA de ciclo de vida
//! (start/stop/input), isolamento por `workspace_id` e um adapter stub.

use crate::models::Agent;
use std::collections::HashMap;
use std::sync::Mutex;

/// Sessão de aplicativo ativa de um agente.
#[derive(Debug, Clone)]
pub struct AppSession {
    pub agent_id: String,
    pub workspace_id: String,
}

/// Fronteira de execução de aplicativo — separada do `RuntimeAdapter` (CLI/PTY).
pub trait AppAgentAdapter: Send + Sync {
    fn name(&self) -> &'static str;

    fn start(&self, agent: &Agent, workspace_id: &str) -> Result<AppSession, String>;

    fn stop(&self, agent_id: &str) -> Result<(), String>;

    fn send_input(&self, agent_id: &str, input: &str) -> Result<(), String>;
}

/// Adapter real (stub) da Fase 15: mantém o ciclo de vida sem automatizar o
/// aplicativo. A automação chega na Fase 16 (Computer Control).
#[derive(Default)]
pub struct StubAppAgentAdapter;

impl AppAgentAdapter for StubAppAgentAdapter {
    fn name(&self) -> &'static str {
        "app_stub"
    }

    fn start(&self, agent: &Agent, workspace_id: &str) -> Result<AppSession, String> {
        Ok(AppSession {
            agent_id: agent.id.clone(),
            workspace_id: workspace_id.to_string(),
        })
    }

    fn stop(&self, _agent_id: &str) -> Result<(), String> {
        Ok(())
    }

    fn send_input(&self, _agent_id: &str, _input: &str) -> Result<(), String> {
        Ok(())
    }
}

/// Gerenciador de sessões de aplicativo, espelhando o `WebSessionManager`.
pub struct AppSessionManager {
    sessions: Mutex<HashMap<String, AppSession>>,
    workspace_of: Mutex<HashMap<String, String>>,
    adapter: Box<dyn AppAgentAdapter>,
}

impl AppSessionManager {
    pub fn new() -> Self {
        Self::with_adapter(Box::new(StubAppAgentAdapter))
    }

    pub fn with_adapter(adapter: Box<dyn AppAgentAdapter>) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            workspace_of: Mutex::new(HashMap::new()),
            adapter,
        }
    }

    pub fn is_running_in(&self, workspace_id: &str, agent_id: &str) -> bool {
        let guard = self
            .workspace_of
            .lock()
            .expect("workspace_of mutex poisoned");
        matches!(guard.get(agent_id), Some(w) if w == workspace_id)
    }

    pub fn start(&self, agent: &Agent, workspace_id: &str) -> Result<AppSession, String> {
        let session = self.adapter.start(agent, workspace_id)?;
        self.sessions
            .lock()
            .expect("sessions mutex poisoned")
            .insert(agent.id.clone(), session.clone());
        self.workspace_of
            .lock()
            .expect("workspace_of mutex poisoned")
            .insert(agent.id.clone(), workspace_id.to_string());
        Ok(session)
    }

    pub fn stop(&self, agent_id: &str) -> Result<(), String> {
        self.adapter.stop(agent_id)?;
        self.sessions
            .lock()
            .expect("sessions mutex poisoned")
            .remove(agent_id);
        self.workspace_of
            .lock()
            .expect("workspace_of mutex poisoned")
            .remove(agent_id);
        Ok(())
    }

    pub fn send_input(&self, agent_id: &str, input: &str) -> Result<(), String> {
        self.adapter.send_input(agent_id, input)
    }

    pub fn stop_all(&self) {
        let ids: Vec<String> = self
            .workspace_of
            .lock()
            .expect("workspace_of mutex poisoned")
            .keys()
            .cloned()
            .collect();
        for id in ids {
            let _ = self.stop(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Agent, AgentKind, Role, Runtime, Status};
    use std::sync::{Arc, Mutex};

    fn app_agent(id: &str) -> Agent {
        Agent {
            id: id.to_string(),
            name: id.to_string(),
            role: Role::Observer,
            runtime: Runtime::Custom,
            kind: AgentKind::App,
            model: String::new(),
            command: String::new(),
            args: vec![],
            working_dir: String::new(),
            auto_start: false,
            status: Status::Idle,
            x: 0.0,
            y: 0.0,
            width: None,
            height: None,
            collapsed: false,
            locked: false,
            accent: None,
        }
    }

    #[test]
    fn lifecycle_start_stop_tracks_session_and_isolation() {
        let manager = AppSessionManager::new();
        let agent = app_agent("a1");
        manager.start(&agent, "ws-a").expect("start");

        assert!(manager.is_running_in("ws-a", "a1"));
        assert!(!manager.is_running_in("ws-b", "a1"));

        manager.stop("a1").expect("stop");
        assert!(!manager.is_running_in("ws-a", "a1"));
    }

    #[test]
    fn stop_all_clears_sessions() {
        let manager = AppSessionManager::new();
        let agent = app_agent("a1");
        manager.start(&agent, "ws-a").expect("start");
        manager.stop_all();
        assert!(!manager.is_running_in("ws-a", "a1"));
    }
}