//! Fase 14 — Agentes Web.
//!
//! `AgentKind::Web` representa um agente que executa num meio web/navegador,
//! separado do meio CLI/PTY. O controle real do navegador é a Fase 16
//! (Computer Control); nesta fase estabelecemos a FRONTEIRA de ciclo de vida
//! (start/stop/input), o isolamento por `workspace_id` e um adapter stub que
//! pode ser substituído por um mock nos testes.
//!
//! A separação espelha o `ProcessManager` (um processo CLI por agente), mas
//! aqui não há processo de SO: a sessão web vive na memória do manager.

use crate::models::Agent;
use std::collections::HashMap;
use std::sync::Mutex;

/// Sessão web ativa de um agente.
#[derive(Debug, Clone)]
pub struct WebSession {
    pub agent_id: String,
    pub workspace_id: String,
}

/// Fronteira de execução web — separada do `RuntimeAdapter` (CLI/PTY).
/// Precisa ser `Send + Sync` porque o manager é compartilhado via `AppState`.
pub trait WebAgentAdapter: Send + Sync {
    /// Nome do adapter (ex.: "web_stub").
    fn name(&self) -> &'static str;

    /// Inicia a sessão web do agente no workspace indicado.
    fn start(&self, agent: &Agent, workspace_id: &str) -> Result<WebSession, String>;

    /// Encerra a sessão web.
    fn stop(&self, agent_id: &str) -> Result<(), String>;

    /// Envia entrada à sessão web (no stub, não há destino ainda).
    fn send_input(&self, agent_id: &str, input: &str) -> Result<(), String>;
}

/// Adapter real (stub) da Fase 14: mantém o ciclo de vida correto sem dirigir
/// um navegador. A implementação que automatiza o navegador chega na Fase 16.
#[derive(Default)]
pub struct StubWebAgentAdapter;

impl WebAgentAdapter for StubWebAgentAdapter {
    fn name(&self) -> &'static str {
        "web_stub"
    }

    fn start(&self, agent: &Agent, workspace_id: &str) -> Result<WebSession, String> {
        Ok(WebSession {
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

/// Gerenciador de sessões web, espelhando o `ProcessManager`:
/// um registro de sessões ativas com isolamento por `workspace_id`.
pub struct WebSessionManager {
    /// Sessões ativas, indexadas por agent id.
    sessions: Mutex<HashMap<String, WebSession>>,
    /// Workspace de cada sessão (agent_id -> workspace_id). Garante que um
    /// stop/input só alcance sessões do workspace atual — nunca de outro.
    workspace_of: Mutex<HashMap<String, String>>,
    /// Adapter de execução web (stub por padrão; mock nos testes).
    adapter: Box<dyn WebAgentAdapter>,
}

impl WebSessionManager {
    pub fn new() -> Self {
        Self::with_adapter(Box::new(StubWebAgentAdapter))
    }

    /// Permite injetar um adapter específico (mock) nos testes.
    pub fn with_adapter(adapter: Box<dyn WebAgentAdapter>) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            workspace_of: Mutex::new(HashMap::new()),
            adapter,
        }
    }

    /// Verifica se o agente possui sessão web ativa PERTENCENTE ao workspace.
    pub fn is_running_in(&self, workspace_id: &str, agent_id: &str) -> bool {
        let guard = self
            .workspace_of
            .lock()
            .expect("workspace_of mutex poisoned");
        matches!(guard.get(agent_id), Some(w) if w == workspace_id)
    }

    /// Inicia a sessão web do agente e a registra com isolamento de workspace.
    pub fn start(&self, agent: &Agent, workspace_id: &str) -> Result<WebSession, String> {
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

    /// Encerra a sessão web do agente (idempotente se não iniciada).
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

    /// Envia entrada à sessão web do agente.
    pub fn send_input(&self, agent_id: &str, input: &str) -> Result<(), String> {
        self.adapter.send_input(agent_id, input)
    }

    /// Encerra todas as sessões web (usado ao trocar de workspace/fechar).
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
    use crate::models::{AgentKind, Agent, Role, Runtime, Status};
    use std::sync::{Arc, Mutex};

    fn web_agent(id: &str) -> Agent {
        Agent {
            id: id.to_string(),
            name: id.to_string(),
            role: Role::Observer,
            runtime: Runtime::Custom,
            kind: AgentKind::Web,
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

    /// Adapter mock que registra as chamadas e nunca falha. Os eventos ficam
    /// num `Arc` compartilhado para inspeção após o uso do manager.
    struct MockWebAgent {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl Default for MockWebAgent {
        fn default() -> Self {
            Self {
                events: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl WebAgentAdapter for MockWebAgent {
        fn name(&self) -> &'static str {
            "web_mock"
        }

        fn start(&self, agent: &Agent, workspace_id: &str) -> Result<WebSession, String> {
            self.events
                .lock()
                .unwrap()
                .push(format!("start:{}:{}", agent.id, workspace_id));
            Ok(WebSession {
                agent_id: agent.id.clone(),
                workspace_id: workspace_id.to_string(),
            })
        }

        fn stop(&self, agent_id: &str) -> Result<(), String> {
            self.events.lock().unwrap().push(format!("stop:{agent_id}"));
            Ok(())
        }

        fn send_input(&self, agent_id: &str, input: &str) -> Result<(), String> {
            self.events
                .lock()
                .unwrap()
                .push(format!("input:{agent_id}:{input}"));
            Ok(())
        }
    }

    #[test]
    fn lifecycle_start_stop_tracks_session() {
        let mock = MockWebAgent::default();
        let events = Arc::clone(&mock.events);
        let manager = WebSessionManager::with_adapter(Box::new(mock));

        let agent = web_agent("w1");
        manager.start(&agent, "ws-a").expect("start");

        assert!(manager.is_running_in("ws-a", "w1"));
        assert!(!manager.is_running_in("ws-b", "w1"));

        manager.stop("w1").expect("stop");
        assert!(!manager.is_running_in("ws-a", "w1"));

        let recorded: Vec<String> = events.lock().unwrap().clone();
        assert_eq!(
            recorded,
            vec!["start:w1:ws-a".to_string(), "stop:w1".to_string()]
        );
    }

    #[test]
    fn isolation_by_workspace() {
        let manager = WebSessionManager::new();
        let agent = web_agent("w1");
        manager.start(&agent, "ws-a").expect("start");
        assert!(manager.is_running_in("ws-a", "w1"));
        // Mesmo agente id num workspace diferente não "vaza".
        assert!(!manager.is_running_in("ws-b", "w1"));

        manager.stop_all();
        assert!(!manager.is_running_in("ws-a", "w1"));
    }

    #[test]
    fn send_input_delegates_to_adapter() {
        let mock = MockWebAgent::default();
        let events = Arc::clone(&mock.events);
        let manager = WebSessionManager::with_adapter(Box::new(mock));
        manager.send_input("w1", "hello").expect("send");
        let recorded: Vec<String> = events.lock().unwrap().clone();
        assert_eq!(recorded, vec!["input:w1:hello".to_string()]);
    }
}