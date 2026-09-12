//! Workflow Engine — execução de cadeia baseada no canvas (Fase 6).
//!
//! Primeira versão REAL e mínima: executa os agentes do workspace em ordem
//! topológica ditada pelas edges (A → B → C), entregando a tarefa passo a
//! passo pelo **Agent Protocol** (RoutedMessage via ContextRouter). Não usa
//! um formato paralelo de workflow; o canvas (Agent + Edge) é a fonte única.
//!
//! Escopo desta versão:
//! - disparo explícito de um fluxo;
//! - estado básico: idle/running/completed/failed;
//! - encerramento quando não há próximo passo;
//! - isolamento por workspace (nunca atravessa workspaces);
//! - reuso do Agent Protocol (identidade, workspace, origem, destino, tipo,
//!   payload e timestamp) + bloqueio de duplicação (via transporte).
//!
//! Fora de escopo: scheduler avançado, paralelismo, retries sofisticados,
//! Skills, MCP, Memory, Supabase, Web/App Agents, Marketplace.

use crate::context_router::{ContextRouter, MessageKind};
use crate::events::{WorkflowEvent, WorkflowEventType};
use crate::models::{Agent, Edge};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tauri::{AppHandle, State};

use crate::state::AppState;

/// Identidade sintética do disparador do workflow (origem da primeira entrega).
pub const WORKFLOW_SENDER: &str = "workflow";

/// Estado básico de uma execução de cadeia.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainStatus {
    Idle,
    Running,
    Completed,
    Failed,
}

impl ChainStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChainStatus::Idle => "idle",
            ChainStatus::Running => "running",
            ChainStatus::Completed => "completed",
            ChainStatus::Failed => "failed",
        }
    }
}

/// Registro de uma entrega realizada pela cadeia.
#[derive(Debug, Clone, Serialize)]
pub struct ChainDelivery {
    pub source: String,
    pub target: String,
    pub delivered: bool,
    pub error: Option<String>,
}

/// Resultado/estado de uma execução de cadeia.
#[derive(Debug, Clone, Serialize)]
pub struct ChainExecution {
    pub id: String,
    pub workspace_id: String,
    pub status: ChainStatus,
    /// Ordem topológica dos agentes (ids) determinada pelas edges.
    pub order: Vec<String>,
    pub deliveries: Vec<ChainDelivery>,
    /// Eventos observáveis emitidos durante a execução (Fase 7).
    pub events: Vec<WorkflowEvent>,
    pub error: Option<String>,
}

/// Ordem topológica (Kahn) dos agentes do workspace a partir das edges
/// direcionadas (source -> target).
///
/// Valida que todo endpoint de edge referencia um agente existente e rejeita
/// ciclos. A ordem é determinística (fila ordenada) para testes estáveis.
pub fn chain_order(agents: &[Agent], edges: &[Edge]) -> Result<Vec<String>, String> {
    let ids: HashSet<&str> = agents.iter().map(|a| a.id.as_str()).collect();

    // Valida destinos/origens (destino inválido => erro).
    for e in edges {
        if !ids.contains(e.source.as_str()) {
            return Err(format!("edge referencia origem inexistente: {}", e.source));
        }
        if !ids.contains(e.target.as_str()) {
            return Err(format!("edge referencia destino inexistente: {}", e.target));
        }
    }

    let mut indegree: HashMap<String, usize> =
        agents.iter().map(|a| (a.id.clone(), 0)).collect();
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    for a in agents {
        adj.entry(a.id.clone()).or_default();
    }
    for e in edges {
        *indegree.entry(e.target.clone()).or_default() += 1;
        adj.entry(e.source.clone()).or_default().push(e.target.clone());
    }

    // Fila determinística: ordena para ordem previsível nos testes.
    let mut queue: VecDeque<String> = indegree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(k, _)| k.clone())
        .collect();
    let mut sorted: Vec<String> = queue.iter().cloned().collect();
    sorted.sort();
    queue = VecDeque::from(sorted);

    let mut order: Vec<String> = Vec::new();
    while let Some(n) = queue.pop_front() {
        order.push(n.clone());
        let mut neigh = adj.get(&n).cloned().unwrap_or_default();
        neigh.sort();
        for m in neigh {
            let d = indegree.get_mut(&m).unwrap();
            *d -= 1;
            if *d == 0 {
                queue.push_back(m);
            }
        }
        // mantém determinismo reordenando após inserções
        let mut items: Vec<String> = queue.iter().cloned().collect();
        items.sort();
        queue = VecDeque::from(items);
    }

    if order.len() != agents.len() {
        return Err("workflow contém ciclo".to_string());
    }

    Ok(order)
}

/// Executa a cadeia: entrega a tarefa aos agentes na ordem topológica das
/// edges, usando o Agent Protocol. Retorna o estado final da execução.
///
/// - `workspace_id`: identifica o canvas (base do isolamento).
/// - `router`: roteador montado sobre o transporte (CLI real ou mock).
/// - `payload`: a tarefa/mensagem disparada explicitamente.
pub fn run_chain(
    workspace_id: &str,
    agents: &[Agent],
    edges: &[Edge],
    router: &ContextRouter,
    app: Option<&AppHandle>,
    payload: &str,
) -> ChainExecution {
    let mut exec = ChainExecution {
        id: crate::workflow::gen_id("chain"),
        workspace_id: workspace_id.to_string(),
        status: ChainStatus::Idle,
        order: Vec::new(),
        deliveries: Vec::new(),
        events: Vec::new(),
        error: None,
    };

    // Resolve a ordem; erro de destino inválido/ciclo => Failed.
    let order = match chain_order(agents, edges) {
        Ok(o) => o,
        Err(e) => {
            exec.status = ChainStatus::Failed;
            exec.error = Some(e.clone());
            exec.events.push(make_event(
                workspace_id,
                &exec.id,
                WorkflowEventType::WorkflowFailed,
                None,
                None,
                Some(e),
            ));
            return exec;
        }
    };
    exec.order = order.clone();

    exec.status = ChainStatus::Running;
    exec.events.push(make_event(
        workspace_id,
        &exec.id,
        WorkflowEventType::WorkflowStarted,
        None,
        None,
        None,
    ));

    if order.is_empty() {
        exec.status = ChainStatus::Completed;
        exec.events.push(make_event(
            workspace_id,
            &exec.id,
            WorkflowEventType::WorkflowCompleted,
            None,
            None,
            None,
        ));
        return exec;
    }

    // Mapa de indegree para identificar os agentes de entrada (fontes).
    let mut indegree: HashMap<String, usize> =
        agents.iter().map(|a| (a.id.clone(), 0)).collect();
    for e in edges {
        *indegree.entry(e.target.clone()).or_default() += 1;
    }

    let mut failed = false;
    let mut failure_step: Option<String> = None;
    let mut failure_data: Option<String> = None;

    // Percorre a ordem topológica, emitindo step_started/message_delivered/
    // step_failed por nó.
    for node_id in &order {
        let is_source = indegree.get(node_id).copied().unwrap_or(0) == 0;

        exec.events.push(make_event(
            workspace_id,
            &exec.id,
            WorkflowEventType::StepStarted,
            Some(node_id),
            Some(node_id),
            None,
        ));

        // Fonte: recebe a tarefa inicial do disparador.
        if is_source {
            let report = router.deliver_direct(
                app,
                workspace_id,
                agents,
                WORKFLOW_SENDER,
                node_id,
                MessageKind::Context,
                payload,
            );
            exec.deliveries.push(ChainDelivery {
                source: WORKFLOW_SENDER.to_string(),
                target: node_id.clone(),
                delivered: report.delivered,
                error: report.error.clone(),
            });
            if report.delivered {
                exec.events.push(make_event(
                    workspace_id,
                    &exec.id,
                    WorkflowEventType::MessageDelivered,
                    Some(node_id),
                    None,
                    Some(payload.to_string()),
                ));
            } else {
                let err = report.error.unwrap_or_else(|| "entrega inicial falhou".to_string());
                exec.events.push(make_event(
                    workspace_id,
                    &exec.id,
                    WorkflowEventType::StepFailed,
                    Some(node_id),
                    Some(node_id),
                    Some(err.clone()),
                ));
                failure_step = Some(node_id.clone());
                failure_data = Some(err);
                failed = true;
            }
        }

        // Propaga ao longo das edges saindo deste nó.
        for e in edges.iter().filter(|e| &e.source == node_id) {
            if failed {
                break;
            }
            let report = router.route_to_peer(
                app,
                workspace_id,
                agents,
                edges,
                node_id,
                &e.target,
                payload,
            );
            exec.deliveries.push(ChainDelivery {
                source: node_id.clone(),
                target: e.target.clone(),
                delivered: report.delivered,
                error: report.error.clone(),
            });
            if report.delivered {
                exec.events.push(make_event(
                    workspace_id,
                    &exec.id,
                    WorkflowEventType::MessageDelivered,
                    Some(&e.target),
                    Some(node_id),
                    Some(payload.to_string()),
                ));
            } else {
                let err = report.error.unwrap_or_else(|| "entrega falhou".to_string());
                exec.events.push(make_event(
                    workspace_id,
                    &exec.id,
                    WorkflowEventType::StepFailed,
                    Some(node_id),
                    Some(node_id),
                    Some(err.clone()),
                ));
                failure_step = Some(node_id.clone());
                failure_data = Some(err);
                failed = true;
                break;
            }
        }

        if failed {
            break;
        }
    }

    if failed {
        exec.status = ChainStatus::Failed;
        let step = failure_step.unwrap();
        exec.error = Some(format!("falha no passo {}", step));
        exec.events.push(make_event(
            workspace_id,
            &exec.id,
            WorkflowEventType::WorkflowFailed,
            Some(&step),
            Some(&step),
            failure_data,
        ));
    } else {
        exec.status = ChainStatus::Completed;
        exec.events.push(make_event(
            workspace_id,
            &exec.id,
            WorkflowEventType::WorkflowCompleted,
            None,
            None,
            None,
        ));
    }

    exec
}

/// Constrói um evento observável com identidade/escopo.
fn make_event(
    workspace_id: &str,
    workflow_id: &str,
    event: WorkflowEventType,
    agent_id: Option<&str>,
    step: Option<&str>,
    data: Option<String>,
) -> WorkflowEvent {
    WorkflowEvent {
        id: crate::workflow::gen_id("evt"),
        workspace_id: workspace_id.to_string(),
        workflow_id: workflow_id.to_string(),
        event,
        agent_id: agent_id.map(|s| s.to_string()),
        step: step.map(|s| s.to_string()),
        data,
        timestamp: crate::workflow::now_millis(),
    }
}

/// Dispara explicitamente um fluxo no workspace atual (canvas como fonte).
#[tauri::command]
pub fn run_canvas_chain(
    app: AppHandle,
    state: State<'_, AppState>,
    payload: String,
) -> Result<ChainExecution, String> {
    let (workspace_id, agents, edges) = {
        let guard = state
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        (
            guard.metadata.id.clone(),
            guard.agents.clone(),
            guard.edges.clone(),
        )
    };

    let transport = Arc::new(crate::context_router::CliContextTransport);
    let router = ContextRouter::new(transport);
    let exec = run_chain(&workspace_id, &agents, &edges, &router, Some(&app), &payload);

    // Registra os eventos na memória e no disco (escopado ao workspace).
    state.record_workflow_events(&workspace_id, &exec.events);

    Ok(exec)
}

/// Lista os eventos de workflow do workspace ATUAL (isolamento por workspace).
#[tauri::command]
pub fn list_workflow_events(state: State<'_, AppState>) -> Vec<WorkflowEvent> {
    let workspace_id = {
        let guard = state
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        guard.metadata.id.clone()
    };
    state.workflow_events.by_workspace(&workspace_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_router::{ContextTransport, RoutedMessage};
    use crate::models::{AgentKind, Role, Runtime, Status};
    use std::sync::Mutex;

    fn test_agent(id: &str, name: &str) -> Agent {
        Agent {
            id: id.to_string(),
            name: name.to_string(),
            role: Role::Builder,
            runtime: Runtime::Custom,
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

    fn test_edge(id: &str, source: &str, target: &str) -> Edge {
        Edge {
            id: id.to_string(),
            source: source.to_string(),
            target: target.to_string(),
            source_handle: None,
            target_handle: None,
            edge_type: crate::models::EdgeType::Message,
            label: None,
        }
    }

    /// Transporte de teste: registra entregas e simula destino não rodando /
    /// isolamento por workspace.
    struct ChainMockTransport {
        delivered: Mutex<Vec<(String, String)>>, // (target, payload)
        running: HashSet<String>,
        workspace: Option<String>,
    }

    impl ChainMockTransport {
        fn new(running: &[&str]) -> Self {
            Self {
                delivered: Mutex::new(Vec::new()),
                running: running.iter().map(|s| s.to_string()).collect(),
                workspace: None,
            }
        }

        fn with_workspace(mut self, ws: &str) -> Self {
            self.workspace = Some(ws.to_string());
            self
        }

        fn deliveries(&self) -> Vec<(String, String)> {
            self.delivered.lock().unwrap().clone()
        }
    }

    impl ContextTransport for ChainMockTransport {
        fn deliver(
            &self,
            _app: Option<&AppHandle>,
            target: &Agent,
            message: &RoutedMessage,
        ) -> Result<(), String> {
            // Isolamento por workspace (simula assert_workspace real).
            if let Some(expected) = &self.workspace {
                if expected != &message.workspace_id {
                    return Err("destino pertence a outro workspace".to_string());
                }
            }
            if !self.running.contains(&target.id) {
                return Err(format!("agente {} não está em execução", target.id));
            }
            self.delivered
                .lock()
                .unwrap()
                .push((target.id.clone(), message.payload.clone()));
            Ok(())
        }
    }

    #[test]
    fn chain_order_respects_edges() {
        let agents = vec![
            test_agent("a1", "OpenCode"),
            test_agent("a2", "Cline"),
            test_agent("a3", "Reviewer"),
        ];
        let edges = vec![
            test_edge("e1", "a1", "a2"),
            test_edge("e2", "a2", "a3"),
        ];
        let order = chain_order(&agents, &edges).unwrap();
        assert_eq!(order, vec!["a1", "a2", "a3"]);
    }

    #[test]
    fn chain_order_rejects_invalid_destination() {
        let agents = vec![test_agent("a1", "OpenCode"), test_agent("a2", "Cline")];
        let edges = vec![test_edge("e1", "a1", "ghost")]; // ghost não existe
        let err = chain_order(&agents, &edges).unwrap_err();
        assert!(err.contains("destino inexistente"));
    }

    #[test]
    fn chain_order_rejects_cycle() {
        let agents = vec![test_agent("a1", "A1"), test_agent("a2", "A2")];
        let edges = vec![test_edge("e1", "a1", "a2"), test_edge("e2", "a2", "a1")];
        assert!(chain_order(&agents, &edges).is_err());
    }

    #[test]
    fn run_chain_a_b_c_completes_and_terminates() {
        let agents = vec![
            test_agent("a1", "OpenCode"),
            test_agent("a2", "Cline"),
            test_agent("a3", "Reviewer"),
        ];
        let edges = vec![
            test_edge("e1", "a1", "a2"),
            test_edge("e2", "a2", "a3"),
        ];
        let transport = Arc::new(ChainMockTransport::new(&["a1", "a2", "a3"]));
        let router = ContextRouter::new(transport);

        let exec = run_chain("ws1", &agents, &edges, &router, None, "tarefa");

        assert_eq!(exec.status, ChainStatus::Completed);
        assert_eq!(exec.order, vec!["a1", "a2", "a3"]);
        // 3 entregas: workflow->a1, a1->a2, a2->a3 (e encerra sem próxima).
        assert_eq!(exec.deliveries.len(), 3);
        assert!(exec.deliveries.iter().all(|d| d.delivered));
        assert_eq!(exec.deliveries[0].target, "a1");
        assert_eq!(exec.deliveries[1].target, "a2");
        assert_eq!(exec.deliveries[2].target, "a3");
    }

    #[test]
    fn run_chain_invalid_destination_fails() {
        let agents = vec![test_agent("a1", "OpenCode"), test_agent("a2", "Cline")];
        let edges = vec![test_edge("e1", "a1", "ghost")];
        let transport = Arc::new(ChainMockTransport::new(&["a1", "a2"]));
        let router = ContextRouter::new(transport);

        let exec = run_chain("ws1", &agents, &edges, &router, None, "x");
        assert_eq!(exec.status, ChainStatus::Failed);
        assert!(exec.error.as_deref().unwrap().contains("destino inexistente"));
    }

    #[test]
    fn run_chain_wrong_workspace_fails() {
        let agents = vec![
            test_agent("a1", "OpenCode"),
            test_agent("a2", "Cline"),
        ];
        let edges = vec![test_edge("e1", "a1", "a2")];
        // O transporte só aceita entregas no workspace ws-A.
        let transport = Arc::new(ChainMockTransport::new(&["a1", "a2"]).with_workspace("ws-A"));
        let router = ContextRouter::new(transport);

        // Executa com workspace errado (ws-B) => todas as entregas falham.
        let exec = run_chain("ws-B", &agents, &edges, &router, None, "x");
        assert_eq!(exec.status, ChainStatus::Failed);
        assert!(exec.error.is_some());
    }

    #[test]
    fn run_chain_agent_failure_stops_chain() {
        let agents = vec![
            test_agent("a1", "OpenCode"),
            test_agent("a2", "Cline"),
            test_agent("a3", "Reviewer"),
        ];
        let edges = vec![
            test_edge("e1", "a1", "a2"),
            test_edge("e2", "a2", "a3"),
        ];
        // a2 não está rodando => a1->a2 falha e a3 não recebe nada.
        let transport = Arc::new(ChainMockTransport::new(&["a1", "a3"]));
        let router = ContextRouter::new(transport);

        let exec = run_chain("ws1", &agents, &edges, &router, None, "x");
        assert_eq!(exec.status, ChainStatus::Failed);
        assert!(exec.error.is_some());

        // a3 nunca é alcançado (cadeia parou na falha de a2).
        let reached = exec
            .deliveries
            .iter()
            .any(|d| d.target == "a3");
        assert!(!reached, "C não deve ser alcançado após falha de B");
    }

    #[test]
    fn run_chain_single_agent_completes() {
        let agents = vec![test_agent("a1", "OpenCode")];
        let edges: Vec<Edge> = vec![];
        let transport = Arc::new(ChainMockTransport::new(&["a1"]));
        let router = ContextRouter::new(transport);

        let exec = run_chain("ws1", &agents, &edges, &router, None, "x");
        assert_eq!(exec.status, ChainStatus::Completed);
        // Sem próximos passos: apenas a entrega inicial.
        assert_eq!(exec.deliveries.len(), 1);
        assert_eq!(exec.deliveries[0].target, "a1");
    }

    #[test]
    fn run_chain_emits_ordered_events() {
        let agents = vec![
            test_agent("a1", "OpenCode"),
            test_agent("a2", "Cline"),
            test_agent("a3", "Reviewer"),
        ];
        let edges = vec![
            test_edge("e1", "a1", "a2"),
            test_edge("e2", "a2", "a3"),
        ];
        let transport = Arc::new(ChainMockTransport::new(&["a1", "a2", "a3"]));
        let router = ContextRouter::new(transport);

        let exec = run_chain("ws1", &agents, &edges, &router, None, "tarefa");

        // Sequência: started ... completed.
        assert_eq!(exec.events[0].event, WorkflowEventType::WorkflowStarted);
        assert_eq!(
            exec.events.last().unwrap().event,
            WorkflowEventType::WorkflowCompleted
        );

        // Escopo correto em todos os eventos.
        assert!(exec
            .events
            .iter()
            .all(|e| e.workspace_id == "ws1" && e.workflow_id == exec.id));

        // Ordem dos steps: a1, a2, a3.
        let steps: Vec<_> = exec
            .events
            .iter()
            .filter(|e| e.event == WorkflowEventType::StepStarted)
            .map(|e| e.step.clone().unwrap())
            .collect();
        assert_eq!(steps, vec!["a1", "a2", "a3"]);

        // Ordem das entregas: workflow->a1, a1->a2, a2->a3.
        let delivered: Vec<_> = exec
            .events
            .iter()
            .filter(|e| e.event == WorkflowEventType::MessageDelivered)
            .map(|e| e.agent_id.clone().unwrap())
            .collect();
        assert_eq!(delivered, vec!["a1", "a2", "a3"]);
    }

    #[test]
    fn run_chain_failure_emits_step_failed_and_workflow_failed() {
        let agents = vec![
            test_agent("a1", "OpenCode"),
            test_agent("a2", "Cline"),
            test_agent("a3", "Reviewer"),
        ];
        let edges = vec![
            test_edge("e1", "a1", "a2"),
            test_edge("e2", "a2", "a3"),
        ];
        // a2 não está rodando => a1->a2 falha, a3 não é alcançado.
        let transport = Arc::new(ChainMockTransport::new(&["a1", "a3"]));
        let router = ContextRouter::new(transport);

        let exec = run_chain("ws1", &agents, &edges, &router, None, "x");

        assert_eq!(exec.status, ChainStatus::Failed);
        assert!(exec
            .events
            .iter()
            .any(|e| e.event == WorkflowEventType::StepFailed));
        assert_eq!(
            exec.events.last().unwrap().event,
            WorkflowEventType::WorkflowFailed
        );
        // a3 nunca recebeu mensagem.
        assert!(!exec.events.iter().any(|e| {
            e.event == WorkflowEventType::MessageDelivered && e.agent_id.as_deref() == Some("a3")
        }));
        // Sem completed.
        assert!(!exec
            .events
            .iter()
            .any(|e| e.event == WorkflowEventType::WorkflowCompleted));
    }

    #[test]
    fn run_chain_invalid_destination_emits_workflow_failed_without_start() {
        let agents = vec![test_agent("a1", "OpenCode"), test_agent("a2", "Cline")];
        let edges = vec![test_edge("e1", "a1", "ghost")];
        let transport = Arc::new(ChainMockTransport::new(&["a1", "a2"]));
        let router = ContextRouter::new(transport);

        let exec = run_chain("ws1", &agents, &edges, &router, None, "x");

        assert_eq!(exec.status, ChainStatus::Failed);
        // Falha no plan (antes de começar): só workflow_failed, sem started.
        assert_eq!(exec.events.len(), 1);
        assert_eq!(exec.events[0].event, WorkflowEventType::WorkflowFailed);
    }
}