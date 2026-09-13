use crate::models::{Agent, AgentKind, Edge, EdgeType, Role, Runtime, Status, Viewport, WorkspaceListItem, WorkspaceSettings, WorkspaceState};
use crate::context_router::{build_peers_banner, pty_submission, supports_agent_protocol, AskReport, CliContextTransport, ContextRouter, DeliveryReport, RequestRegistry};
use crate::events::{WorkflowEvent, WorkflowEventLog};
use crate::persistence::Persistence;
use crate::process_manager::ProcessManager;
use crate::workflow::{WorkflowDefinition, WorkflowExecution};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};


#[derive(Debug, Clone)]
pub struct AgentBusMessage {
    pub workspace_id: String,
    pub agent_id: String,
    pub direction: String,
    pub data: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone)]
pub struct AgentBusState {
    pub messages: Arc<Mutex<Vec<AgentBusMessage>>>,
}

impl AgentBusState {
    pub fn new() -> Self {
        Self {
            messages: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn push(&self, message: AgentBusMessage) {
        const LIMIT: usize = 500;

        let mut guard = self.messages.lock().expect("agent bus mutex poisoned");
        guard.push(message);

        if guard.len() > LIMIT {
            let excess = guard.len() - LIMIT;
            guard.drain(0..excess);
        }
    }

    pub fn get_agent_messages(&self, workspace_id: &str, agent_id: &str) -> Vec<AgentBusMessage> {
        let guard = self.messages.lock().expect("agent bus mutex poisoned");

        guard
            .iter()
            .filter(|m| m.workspace_id == workspace_id && m.agent_id == agent_id)
            .cloned()
            .collect()
    }
}

impl Default for AgentBusState {
    fn default() -> Self {
        Self::new()
    }
}

/// Estado da aplicação em memória + persistência em JSON.
/// O backend é a fonte da verdade: toda mutação passa por aqui
/// e é gravada em disco imediatamente.
pub struct AppState {
    /// Workspace atualmente carregado.
    pub workspace: Mutex<WorkspaceState>,
    /// ID do workspace atualmente carregado.
    pub current_workspace_id: Mutex<Option<String>>,
    /// Workflows definidos (por ID).
    pub workflows: Mutex<HashMap<String, WorkflowDefinition>>,
    /// Execuções em andamento/histórico (por ID).
    pub executions: Mutex<HashMap<String, WorkflowExecution>>,
    pub persistence: Persistence,
    pub processes: ProcessManager,

    /// Barramento interno de comunicação entre agentes.
    pub agent_bus: AgentBusState,

    /// Ids de mensagens já entregues (bloqueio de duplicação do Agent Protocol).
    pub delivered_ids: Mutex<HashSet<String>>,

    /// Handshakes confirmados pelo canal IPC (`workspace_id\0agent_id`).
    /// Controla o retry de discovery sem continuar injetando prompts no TUI.
    pub protocol_ready: Mutex<HashSet<String>>,

    /// Pedidos (ask) pendentes aguardando resposta (correlação request/reply).
    pub pending_requests: RequestRegistry,

    /// Log de eventos de workflow observáveis (Fase 7), escopado por workspace.
    pub workflow_events: WorkflowEventLog,
}

impl AppState {
    /// Cria o estado carregando o workspace persistido (se existir).
    /// Caso contrário, gera o estado inicial com agentes de exemplo.
    pub fn new(app_data_dir: std::path::PathBuf) -> Self {
        let persistence = Persistence::new(app_data_dir);

        // Migra o formato legado (workspace.json v1) para multi-workspace,
        // se necessário. Falhas de migração não devem impedir o boot.
        let _ = persistence.migrate_legacy();

        // Tenta carregar o último workspace aberto
        let workspace = persistence
            .load_last_workspace()
            .ok()
            .flatten()
            .unwrap_or_else(|| initial_workspace());

        // Processos não sobrevivem ao restart do app: qualquer status
        // "ativo" persistido é resetado para Stopped.
        let mut workspace = workspace;
        let mut reset_any = false;
        for agent in &mut workspace.agents {
            match agent.status {
                Status::Running | Status::Starting | Status::Waiting => {
                    agent.status = Status::Stopped;
                    reset_any = true;
                }
                _ => {}
            }
        }
        // Persiste o reset para o JSON não ficar com status fantasma.
        if reset_any {
            let _ = persistence.save_workspace(&workspace);
        }

        let workspace_id = workspace.metadata.id.clone();

        // Hidrata o log de eventos a partir do disco (recuperação após reinício).
        let mut workflow_events = WorkflowEventLog::new();
        workflow_events.extend(persistence.load_all_events().unwrap_or_default());

        Self {
            workspace: Mutex::new(workspace),
            current_workspace_id: Mutex::new(Some(workspace_id)),
            workflows: Mutex::new(HashMap::new()),
            executions: Mutex::new(HashMap::new()),
            persistence,
            processes: ProcessManager::new(),
            agent_bus: AgentBusState::new(),
            delivered_ids: Mutex::new(HashSet::new()),
            protocol_ready: Mutex::new(HashSet::new()),
            pending_requests: RequestRegistry::new(),
            workflow_events,
        }
    }

    /// Carrega um workspace específico por ID.
    pub fn load_workspace(&self, workspace_id: &str) -> Result<WorkspaceState, String> {
        self.persistence.load_workspace(workspace_id)?
            .ok_or_else(|| "workspace não encontrado".to_string())
    }

    /// Salva o workspace atual.
    pub fn persist(&self) -> Result<(), String> {
        let guard = self
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        self.persistence.save_workspace(&guard)
    }

    /// Marca o workspace como modificado (atualiza `updated_at`).
    pub fn touch(&self) {
        let mut guard = self
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        guard.touch();
    }

    /// Retorna o ID do workspace atual.
    pub fn current_workspace_id(&self) -> Option<String> {
        self.current_workspace_id.lock().expect("workspace_id mutex poisoned").clone()
    }

    /// Define o workspace atual.
    pub fn set_current_workspace(&self, workspace: WorkspaceState) {
        let workspace_id = workspace.metadata.id.clone();
        {
            let mut guard = self.workspace.lock().expect("workspace mutex poisoned");
            *guard = workspace;
        }
        {
            let mut id = self.current_workspace_id.lock().expect("workspace_id mutex poisoned");
            *id = Some(workspace_id);
        }
    }

    /// Para TODOS os processos e reseta os status ativos do workspace atual
    /// para Stopped, persistindo o resultado. Deve ser chamado antes de
    /// trocar/deletar workspace para não deixar processos órfãos nem
    /// status "running" fantasma no JSON.
    pub fn stop_all_processes(&self) {
        self.processes.stop_all();
        let mut changed = false;
        {
            let mut guard = self.workspace.lock().expect("workspace mutex poisoned");
            for agent in &mut guard.agents {
                match agent.status {
                    Status::Running | Status::Starting | Status::Waiting => {
                        agent.status = Status::Stopped;
                        changed = true;
                    }
                    _ => {}
                }
            }
        }
        if changed {
            let _ = self.persist();
        }
    }

    /// Roteia um payload da origem para os destinos conectados (Fase 3).
    /// Resolve as edges de saída e delega a entrega ao transporte (CLI hoje).
    /// Também registra as entregas no barramento interno (Fase 4) para que o
    /// histórico de comunicação A -> B fique observável via `bus_get_agent`.
    pub fn route_context_from(
        &self,
        app: &tauri::AppHandle,
        source_id: &str,
        payload: &str,
    ) -> Vec<DeliveryReport> {
        let (workspace_id, agents, edges) = {
            let guard = self.workspace.lock().expect("workspace mutex poisoned");
            (
                guard.metadata.id.clone(),
                guard.agents.clone(),
                guard.edges.clone(),
            )
        };
        let transport = Arc::new(CliContextTransport);
        let router = ContextRouter::new(transport);
        let reports = router.route(Some(app), &workspace_id, &agents, &edges, source_id, payload);

        // Registra a comunicação no barramento interno (out = origem, in = destino).
        let ts = crate::agent_bus::now_ms();
        self.agent_bus.push(AgentBusMessage {
            workspace_id: workspace_id.clone(),
            agent_id: source_id.to_string(),
            direction: "out".to_string(),
            data: payload.to_string(),
            timestamp: ts,
        });
        for report in &reports {
            if report.delivered {
                self.agent_bus.push(AgentBusMessage {
                    workspace_id: workspace_id.clone(),
                    agent_id: report.target.clone(),
                    direction: "in".to_string(),
                    data: payload.to_string(),
                    timestamp: ts,
                });
            }
        }

        reports
    }

    /// Roteia um payload da origem para um peer específico (nome OU id),
    /// restrito à topologia das edges (Fase 4) — envio iniciado pelo próprio
    /// agente, sem depender de filesystem/processos/internet.
    pub fn route_context_to_peer(
        &self,
        app: &tauri::AppHandle,
        source_id: &str,
        target_ref: &str,
        payload: &str,
    ) -> DeliveryReport {
        let (workspace_id, agents, edges) = {
            let guard = self.workspace.lock().expect("workspace mutex poisoned");
            (
                guard.metadata.id.clone(),
                guard.agents.clone(),
                guard.edges.clone(),
            )
        };
        let transport = Arc::new(CliContextTransport);
        let router = ContextRouter::new(transport);
        let report =
            router.route_to_peer(Some(app), &workspace_id, &agents, &edges, source_id, target_ref, payload);

        // Registra a comunicação no barramento interno (out = origem, in = destino).
        let ts = crate::agent_bus::now_ms();
        self.agent_bus.push(AgentBusMessage {
            workspace_id: workspace_id.clone(),
            agent_id: source_id.to_string(),
            direction: "out".to_string(),
            data: payload.to_string(),
            timestamp: ts,
        });
        if report.delivered {
            self.agent_bus.push(AgentBusMessage {
                workspace_id: workspace_id.clone(),
                agent_id: report.target.clone(),
                direction: "in".to_string(),
                data: payload.to_string(),
                timestamp: ts,
            });
        }

        report
    }

    /// Faz um pedido (ask) de `source_id` a um peer (nome ou id), restrito à
    /// topologia das edges e ao workspace atual. A request fica pendente para
    /// correlação da resposta (request/reply) com timeout.
    pub fn ask_peer(
        &self,
        app: &tauri::AppHandle,
        source_id: &str,
        target_ref: &str,
        payload: &str,
    ) -> AskReport {
        let (workspace_id, agents, edges) = {
            let guard = self.workspace.lock().expect("workspace mutex poisoned");
            (
                guard.metadata.id.clone(),
                guard.agents.clone(),
                guard.edges.clone(),
            )
        };
        let now = crate::agent_bus::now_ms();
        // Higieniza pedidos expirados antes de registrar um novo (sem vazamento).
        self.pending_requests.prune_expired(now);
        let transport = Arc::new(CliContextTransport);
        let router = ContextRouter::new(transport);
        let report = router.ask_to_peer(
            Some(app),
            &self.pending_requests,
            &workspace_id,
            &agents,
            &edges,
            source_id,
            target_ref,
            payload,
            now,
        );

        // Registra o pedido no barramento interno (out = origem, in = destino).
        self.agent_bus.push(AgentBusMessage {
            workspace_id: workspace_id.clone(),
            agent_id: source_id.to_string(),
            direction: "ask_out".to_string(),
            data: payload.to_string(),
            timestamp: now,
        });
        if report.delivered {
            self.agent_bus.push(AgentBusMessage {
                workspace_id: workspace_id.clone(),
                agent_id: report.target.clone(),
                direction: "ask_in".to_string(),
                data: payload.to_string(),
                timestamp: now,
            });
        }

        report
    }

    /// Processa uma resposta (reply) de `source_id` correlacionada a uma
    /// request. Valida workspace/emissor/timeout e entrega a `Reply` ao
    /// requisitante original.
    pub fn reply_to_request(
        &self,
        app: &tauri::AppHandle,
        source_id: &str,
        correlation_id: &str,
        payload: &str,
    ) -> DeliveryReport {
        let (workspace_id, agents) = {
            let guard = self.workspace.lock().expect("workspace mutex poisoned");
            (guard.metadata.id.clone(), guard.agents.clone())
        };
        let now = crate::agent_bus::now_ms();
        let transport = Arc::new(CliContextTransport);
        let router = ContextRouter::new(transport);
        let report = router.reply_to_request(
            Some(app),
            &self.pending_requests,
            &workspace_id,
            &agents,
            source_id,
            correlation_id,
            payload,
            now,
        );

        // Registra a resposta no barramento interno.
        self.agent_bus.push(AgentBusMessage {
            workspace_id: workspace_id.clone(),
            agent_id: source_id.to_string(),
            direction: "reply_out".to_string(),
            data: payload.to_string(),
            timestamp: now,
        });
        if report.delivered {
            self.agent_bus.push(AgentBusMessage {
                workspace_id: workspace_id.clone(),
                agent_id: report.target.clone(),
                direction: "reply_in".to_string(),
                data: payload.to_string(),
                timestamp: now,
            });
        }

        report
    }

    /// Remove pedidos pendentes expirados (timeout). Retorna quantos removeu.
    /// Evita crescimento sem limite do registro de correlação.
    pub fn prune_expired_requests(&self, now: u64) -> usize {
        self.pending_requests.prune_expired(now)
    }

    fn protocol_ready_key(workspace_id: &str, agent_id: &str) -> String {
        format!("{workspace_id}\0{agent_id}")
    }

    pub fn mark_protocol_ready(&self, workspace_id: &str, agent_id: &str) {
        self.protocol_ready
            .lock()
            .expect("protocol_ready mutex poisoned")
            .insert(Self::protocol_ready_key(workspace_id, agent_id));
    }

    pub fn clear_protocol_ready(&self, workspace_id: &str, agent_id: &str) {
        self.protocol_ready
            .lock()
            .expect("protocol_ready mutex poisoned")
            .remove(&Self::protocol_ready_key(workspace_id, agent_id));
    }

    pub fn is_protocol_ready(&self, workspace_id: &str, agent_id: &str) -> bool {
        self.protocol_ready
            .lock()
            .expect("protocol_ready mutex poisoned")
            .contains(&Self::protocol_ready_key(workspace_id, agent_id))
    }

    /// Nomes dos peers conectados no canvas atual, para resposta do bridge.
    pub fn protocol_peer_names(&self, agent_id: &str) -> Vec<String> {
        let guard = self.workspace.lock().expect("workspace mutex poisoned");
        crate::context_router::connected_peer_ids(&guard.edges, agent_id)
            .into_iter()
            .filter_map(|id| guard.agents.iter().find(|agent| agent.id == id))
            .map(|agent| agent.name.clone())
            .collect()
    }

    /// Notifica um agente (se estiver em execução) com a lista atualizada de
    /// seus peers conectados. Usado na descoberta ao iniciar e ao criar/remover
    /// edges — a topologia do canvas é a fonte de verdade que chega ao agente.
    pub fn announce_peers_to(&self, agent_id: &str) {
        let (workspace_id, banner) = {
            let guard = self.workspace.lock().expect("workspace mutex poisoned");
            let Some(agent) = guard.agents.iter().find(|agent| agent.id == agent_id) else {
                return;
            };
            if !supports_agent_protocol(agent) {
                return;
            }
            (
                guard.metadata.id.clone(),
                build_peers_banner(&guard.agents, &guard.edges, agent_id),
            )
        };
        // A notícia de identidade é sempre enviada (mesmo sem peers): todo
        // agente CLI precisa saber que roda dentro do MAESTRO 2.0.
        if self.processes.is_running_in(&workspace_id, agent_id) {
            let _ = self
                .processes
                .send_input_in(&workspace_id, agent_id, &pty_submission(&banner));
        }
    }

    /// Verifica se uma mensagem (por id) já foi entregue nesta sessão
    /// (Agent Protocol: bloqueio de duplicação).
    pub fn was_delivered(&self, id: &str) -> bool {
        self.delivered_ids
            .lock()
            .expect("delivered_ids mutex poisoned")
            .contains(id)
    }

    /// Marca a mensagem como entregue. Usado somente após a entrega bem-sucedida.
    pub fn mark_delivered(&self, id: &str) {
        let mut set = self.delivered_ids.lock().expect("delivered_ids mutex poisoned");
        if set.len() >= 4096 {
            set.clear(); // limita o crescimento da memória
        }
        set.insert(id.to_string());
    }

    /// Registra eventos de workflow na memória e no disco (persistência
    /// local, escopada por workspace). Falhas de disco não derrubam a execução.
    pub fn record_workflow_events(&self, workspace_id: &str, events: &[WorkflowEvent]) {
        self.workflow_events.extend(events.to_vec());
        let _ = self.persistence.append_events(workspace_id, events);
    }

    // ==================== WORKFLOW METHODS ====================

    /// Lista todos os workflows.
    pub fn list_workflows(&self) -> Vec<WorkflowDefinition> {
        self.workflows.lock().unwrap().values().cloned().collect()
    }

    /// Obtém um workflow por ID.
    pub fn get_workflow(&self, id: &str) -> Option<WorkflowDefinition> {
        self.workflows.lock().unwrap().get(id).cloned()
    }

    /// Cria um novo workflow.
    pub fn create_workflow(&self, mut workflow: WorkflowDefinition) -> WorkflowDefinition {
        workflow.id = crate::workflow::gen_id("wf");
        let now = crate::workflow::now_millis();
        workflow.created_at = now;
        workflow.updated_at = now;
        self.workflows.lock().unwrap().insert(workflow.id.clone(), workflow.clone());
        workflow
    }

    /// Atualiza um workflow existente.
    pub fn update_workflow(&self, workflow: WorkflowDefinition) -> Result<WorkflowDefinition, String> {
        let mut guard = self.workflows.lock().unwrap();
        if !guard.contains_key(&workflow.id) {
            return Err("workflow não encontrado".to_string());
        }
        let mut wf = workflow;
        wf.updated_at = crate::workflow::now_millis();
        guard.insert(wf.id.clone(), wf.clone());
        Ok(wf)
    }

    /// Deleta um workflow.
    pub fn delete_workflow(&self, id: &str) -> Result<(), String> {
        let mut guard = self.workflows.lock().unwrap();
        if guard.remove(id).is_none() {
            return Err("workflow não encontrado".to_string());
        }
        Ok(())
    }

    // ==================== EXECUTION METHODS ====================

    /// Lista todas as execuções.
    pub fn list_executions(&self) -> Vec<WorkflowExecution> {
        self.executions.lock().unwrap().values().cloned().collect()
    }

    /// Obtém uma execução por ID.
    pub fn get_execution(&self, id: &str) -> Option<WorkflowExecution> {
        self.executions.lock().unwrap().get(id).cloned()
    }

    /// Adiciona/atualiza uma execução.
    pub fn upsert_execution(&self, execution: WorkflowExecution) {
        self.executions.lock().unwrap().insert(execution.id.clone(), execution);
    }

    /// Remove execuções antigas (mantém últimas N).
    pub fn prune_executions(&self, keep: usize) {
        let mut guard = self.executions.lock().unwrap();
        if guard.len() <= keep {
            return;
        }
        let mut execs: Vec<_> = guard.values().cloned().collect();
        execs.sort_by_key(|e| e.started_at.unwrap_or(0));
        let to_remove = execs.len() - keep;
        for e in execs.into_iter().take(to_remove) {
            guard.remove(&e.id);
        }
    }
}

/// Estado inicial usado apenas na primeira execução (sem arquivo salvo).
fn initial_workspace() -> WorkspaceState {
    let agents = vec![
        Agent {
            id: "agent-1".to_string(),
            name: "KILO 1".to_string(),
            role: Role::Orchestrator,
            runtime: Runtime::Kilo,
            kind: AgentKind::Cli,
            model: "kilo".to_string(),
            command: "kilo".to_string(),
            args: vec![],
            working_dir: "".to_string(),
            status: Status::Idle,
            x: 0.5,
            y: 0.2,
            width: None,
            height: None,
            collapsed: false,
            locked: false,
            accent: None,
            auto_start: true, // Orchestrator inicia automaticamente
        },
        Agent {
            id: "agent-2".to_string(),
            name: "Builder".to_string(),
            role: Role::Builder,
            runtime: Runtime::ClaudeCode,
            kind: AgentKind::Cli,
            model: "deepseek-v4-pro".to_string(),
            command: "claude".to_string(),
            args: vec![],
            working_dir: "".to_string(),
            status: Status::Idle,
            x: 0.25,
            y: 0.6,
            width: None,
            height: None,
            collapsed: false,
            locked: false,
            accent: None,
            auto_start: false,
        },
        Agent {
            id: "agent-3".to_string(),
            name: "Reviewer".to_string(),
            role: Role::Reviewer,
            runtime: Runtime::ClaudeCode,
            kind: AgentKind::Cli,
            model: "deepseek-v4-pro".to_string(),
            command: "claude".to_string(),
            args: vec![],
            working_dir: "".to_string(),
            status: Status::Idle,
            x: 0.75,
            y: 0.6,
            width: None,
            height: None,
            collapsed: false,
            locked: false,
            accent: None,
            auto_start: false,
        },
    ];

    let mut workspace = WorkspaceState::new("My Workspace".to_string());
    workspace.agents = agents;
    workspace
}
