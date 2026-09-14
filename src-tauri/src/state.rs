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

    /// Sessões web ativas (Fase 14 — AgentKind::Web), isoladas por workspace.
    pub web_sessions: crate::web_agent::WebSessionManager,

    /// Sessões de aplicativo ativas (Fase 15 — AgentKind::App), isoladas por workspace.
    pub app_sessions: crate::app_agent::AppSessionManager,

    /// Registro de permissões de segurança (Fase 17), escopado por workspace.
    pub permissions: crate::permissions::PermissionRegistry,

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

        // Hidrata as permissões de segurança persistidas.
        let permissions = crate::permissions::PermissionRegistry::new();
        permissions.load(&persistence.load_all_permissions().unwrap_or_default());

        Self {
            workspace: Mutex::new(workspace),
            current_workspace_id: Mutex::new(Some(workspace_id)),
            workflows: Mutex::new(HashMap::new()),
            executions: Mutex::new(HashMap::new()),
            persistence,
            processes: ProcessManager::new(),
            web_sessions: crate::web_agent::WebSessionManager::new(),
            app_sessions: crate::app_agent::AppSessionManager::new(),
            permissions,
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
        self.web_sessions.stop_all();
        self.app_sessions.stop_all();
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

    /// Inicia a sessão de um agente não-CLI (Web ou App) no workspace atual.
    /// Agentes CLI não usam sessão e retornam erro (não devem chegar aqui).
    pub fn start_non_cli_session(&self, agent: &Agent, workspace_id: &str) -> Result<(), String> {
        match agent.kind {
            AgentKind::Web => self.web_sessions.start(agent, workspace_id).map(|_| ()),
            AgentKind::App => self.app_sessions.start(agent, workspace_id).map(|_| ()),
            AgentKind::Cli => Err("agente CLI não usa sessão de execução".to_string()),
        }
    }

    /// Encerra a sessão de um agente não-CLI (Web ou App).
    pub fn stop_non_cli_session(&self, agent: &Agent) -> Result<(), String> {
        match agent.kind {
            AgentKind::Web => self.web_sessions.stop(&agent.id),
            AgentKind::App => self.app_sessions.stop(&agent.id),
            AgentKind::Cli => Err("agente CLI não usa sessão de execução".to_string()),
        }
    }

    /// Envia entrada à sessão de um agente não-CLI (Web ou App).
    pub fn send_input_non_cli(
        &self,
        agent_id: &str,
        kind: AgentKind,
        input: &str,
    ) -> Result<(), String> {
        match kind {
            AgentKind::Web => self.web_sessions.send_input(agent_id, input),
            AgentKind::App => self.app_sessions.send_input(agent_id, input),
            AgentKind::Cli => Ok(()),
        }
    }

    /// Registra um evento de comunicação no histórico persistido (Fase 18).
    /// Best-effort: falhas de disco não afetam a execução.
    pub fn record_history(&self, workspace_id: &str, agent_id: &str, direction: &str, data: &str) {
        crate::history::append(&self.persistence, workspace_id, agent_id, direction, data);
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
        self.record_history(&workspace_id, source_id, "out", payload);
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
        self.record_history(&workspace_id, source_id, "out", payload);
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
        self.record_history(&workspace_id, source_id, "ask_out", payload);
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
        self.record_history(&workspace_id, source_id, "reply_out", payload);
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

    /// Tenta interpretar e executar uma intenção de conexão nativa a partir
    /// da entrada do usuário (ex.: "conecta Kilo 1 ao OpenCode 1").
    /// Retorna `Some(())` se a intenção foi reconhecida e tratada nativamente
    /// (não deve ser encaminhada ao CLI), ou `None` se não for uma intenção
    /// de conexão reconhecida (deve seguir fluxo normal para o CLI).
    pub fn handle_connection_intent(
        &self,
        source_agent_id: &str,
        input: &str,
    ) -> Result<Option<()>, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        // Padrões de intenção de conexão, compilados uma única vez (evita
        // recompilar regex a cada tecla digitada no terminal).
        static PATTERNS: std::sync::LazyLock<Vec<regex::Regex>> =
            std::sync::LazyLock::new(|| {
                [
                    // pt-BR
                    r"(?i)^conecta\s+(.+?)\s+(?:ao|a|com)\s+(.+)$",
                    r"(?i)^conectar\s+(.+?)\s+(?:ao|a|com)\s+(.+)$",
                    r"(?i)^ligar\s+(.+?)\s+(?:a|com)\s+(.+)$",
                    // en-US
                    r"(?i)^connect\s+(.+?)\s+(?:to|with)\s+(.+)$",
                    r"(?i)^link\s+(.+?)\s+(?:to|with)\s+(.+)$",
                ]
                .iter()
                .filter_map(|pat| regex::Regex::new(pat).ok())
                .collect()
            });

        let (source_name, target_name) = match PATTERNS
            .iter()
            .find_map(|re| re.captures(trimmed))
        {
            Some(caps) => {
                let src = caps.get(1).map(|m| m.as_str().trim()).unwrap_or("");
                let tgt = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
                if src.is_empty() || tgt.is_empty() {
                    return Ok(None);
                }
                (src.to_string(), tgt.to_string())
            }
            None => return Ok(None),
        };

        // Resolve nomes para IDs dos agentes no workspace atual
        let (agents, edges) = {
            let guard = self.workspace.lock().expect("workspace mutex poisoned");
            (guard.agents.clone(), guard.edges.clone())
        };

        let source_agent = agents.iter().find(|a| a.id == source_agent_id);
        let Some(source_agent) = source_agent else {
            return Ok(None);
        };

        // Resolve source_agent_id -> nome normalizado para comparação
        let source_name_normalized = source_agent.name.trim().to_lowercase();
        let source_name_input = source_name.trim().to_lowercase();

        // O usuário pode referenciar o agente origem pelo nome ou "eu"/"me"/"this"
        let is_self_ref = source_name_input == "eu"
            || source_name_input == "me"
            || source_name_input == "this"
            || source_name_input == source_name_normalized;

        if !is_self_ref && source_name_input != source_name_normalized {
            // O usuário referenciou outro agente como origem - não interceptamos
            // (pode ser um comando para outro agente)
            return Ok(None);
        }

        // Resolve target pelo nome
        let target_agent = agents.iter().find(|a| {
            a.name.trim().to_lowercase() == target_name.trim().to_lowercase()
        });
        let Some(target_agent) = target_agent else {
            // Agente alvo não encontrado - não intercepta, deixa ir para o CLI
            return Ok(None);
        };

        // Verifica se já existe edge entre source e target
        let edge_exists = edges.iter().any(|e| {
            (e.source == source_agent.id && e.target == target_agent.id)
                || (e.source == target_agent.id && e.target == source_agent.id)
        });

        // Se não existe edge, cria
        if !edge_exists {
            let edge = crate::models::Edge {
                id: format!("edge-{}", crate::agent_bus::now_ms()),
                source: source_agent.id.clone(),
                target: target_agent.id.clone(),
                source_handle: None,
                target_handle: None,
                edge_type: crate::models::EdgeType::Message,
                label: None,
            };
            {
                let mut guard = self.workspace.lock().expect("workspace mutex poisoned");
                guard.edges.push(edge);
            }
            self.persist().map_err(|e| format!("falha ao persistir edge: {e}"))?;
        }

        // Handshake nativo: anuncia peers para ambos os agentes (se suportam protocolo)
        // Isso injeta o contexto de peers via bridge IPC da Fase 4
        self.announce_peers_to(&source_agent.id);
        self.announce_peers_to(&target_agent.id);

        Ok(Some(()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Agent, AgentKind, Role, Runtime, Status};

    fn cli_agent(id: &str, name: &str, runtime: Runtime) -> Agent {
        Agent {
            id: id.to_string(),
            name: name.to_string(),
            role: Role::Orchestrator,
            runtime,
            kind: AgentKind::Cli,
            model: String::new(),
            command: "sh".to_string(),
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

    fn test_state() -> AppState {
        // Diretório único por instância para que testes paralelos não
        // compartilhem o mesmo arquivo de workspace (evita corrida no JSON).
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "maestro2-test-{}-{}",
            std::process::id(),
            n
        ));
        let state = AppState::new(dir);
        {
            let mut guard = state.workspace.lock().expect("workspace mutex poisoned");
            guard.agents = vec![
                cli_agent("agent-1", "Kilo 1", Runtime::Kilo),
                cli_agent("agent-2", "OpenCode 1", Runtime::OpenCode),
            ];
        }
        state
    }

    #[test]
    fn connection_intent_creates_edge_and_returns_handled() {
        let state = test_state();

        // "conecta Kilo 1 ao OpenCode 1" partindo do próprio Kilo 1
        let result = state
            .handle_connection_intent("agent-1", "conecta Kilo 1 ao OpenCode 1")
            .expect("no error");
        assert_eq!(result, Some(()));

        let guard = state.workspace.lock().expect("workspace mutex poisoned");
        assert_eq!(guard.edges.len(), 1);
        assert_eq!(guard.edges[0].source, "agent-1");
        assert_eq!(guard.edges[0].target, "agent-2");
    }

    #[test]
    fn connection_intent_accepts_self_reference() {
        let state = test_state();
        let result = state
            .handle_connection_intent("agent-1", "conecta eu ao OpenCode 1")
            .expect("no error");
        assert_eq!(result, Some(()));

        let guard = state.workspace.lock().expect("workspace mutex poisoned");
        assert_eq!(guard.edges.len(), 1);
        assert_eq!(guard.edges[0].source, "agent-1");
        assert_eq!(guard.edges[0].target, "agent-2");
    }

    #[test]
    fn connection_intent_does_not_duplicate_edge() {
        let state = test_state();
        let _ = state.handle_connection_intent("agent-1", "conecta Kilo 1 ao OpenCode 1");
        let _ = state.handle_connection_intent("agent-1", "conecta Kilo 1 ao OpenCode 1");

        let guard = state.workspace.lock().expect("workspace mutex poisoned");
        assert_eq!(guard.edges.len(), 1);
    }

    #[test]
    fn non_connection_input_is_not_handled() {
        let state = test_state();
        // Entrada normal (não-intenção de conexão) não deve ser interceptada.
        let result = state
            .handle_connection_intent("agent-1", "escreva um teste")
            .expect("no error");
        assert_eq!(result, None);

        let guard = state.workspace.lock().expect("workspace mutex poisoned");
        assert!(guard.edges.is_empty());
    }

    #[test]
    fn connection_intent_unknown_target_is_not_handled() {
        let state = test_state();
        let result = state
            .handle_connection_intent("agent-1", "conecta Kilo 1 ao Agent Fantasma")
            .expect("no error");
        assert_eq!(result, None);

        let guard = state.workspace.lock().expect("workspace mutex poisoned");
        assert!(guard.edges.is_empty());
    }

    #[test]
    fn connection_intent_english_variant() {
        let state = test_state();
        let result = state
            .handle_connection_intent("agent-1", "connect Kilo 1 to OpenCode 1")
            .expect("no error");
        assert_eq!(result, Some(()));

        let guard = state.workspace.lock().expect("workspace mutex poisoned");
        assert_eq!(guard.edges.len(), 1);
        assert_eq!(guard.edges[0].source, "agent-1");
        assert_eq!(guard.edges[0].target, "agent-2");
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
