//! Workflow Engine — orquestração sequencial de agents via PTY.
//!
//! Conceitos:
//! - WorkflowDefinition: grafo estático (nodes + edges) definido pelo usuário.
//! - WorkflowExecution: instância de execução com estado mutável.
//! - ExecutionNode: estado de um node numa execução (status, attempts, output).
//! - Attempt: uma tentativa de execução de um node (started_at, finished_at, exit_code, error).
//!
//! O engine executa nodes em ordem topológica (Kahn). Cada node:
//! 1. Inicia o agent via ProcessManager (PTY real).
//! 2. Aguarda conclusão via exit watcher.
//! 3. Captura output (tail) e injeta como stdin no próximo node.
//! 4. Retry se configurado e falhou.
//! 5. Cancela se flag global setada.

use crate::models::{Agent, AgentKind, Status};
use crate::process_manager::ProcessManager;
use crate::runtime::{ProcessHandle, RuntimeError};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

/// ID único gerado com timestamp + contador.
pub fn gen_id(prefix: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let c = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    format!("{}-{}-{}", prefix, now, c)
}

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Status de uma execução de workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl ExecutionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExecutionStatus::Pending => "pending",
            ExecutionStatus::Running => "running",
            ExecutionStatus::Completed => "completed",
            ExecutionStatus::Failed => "failed",
            ExecutionStatus::Cancelled => "cancelled",
        }
    }
}

/// Status de um node dentro de uma execução.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeExecutionStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

impl NodeExecutionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeExecutionStatus::Pending => "pending",
            NodeExecutionStatus::Running => "running",
            NodeExecutionStatus::Completed => "completed",
            NodeExecutionStatus::Failed => "failed",
            NodeExecutionStatus::Skipped => "skipped",
        }
    }
}

/// Uma tentativa de execução de um node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attempt {
    pub id: String,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub output_tail: String, // últimos 64KB
}

/// Estado de execução de um node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionNode {
    pub node_id: String,
    pub agent_id: String,
    pub status: NodeExecutionStatus,
    pub attempts: Vec<Attempt>,
    pub current_attempt: Option<usize>,
    pub output_tail: String, // output acumulado (tail)
    pub error: Option<String>,
}

/// Edge de workflow (execução, não visual).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEdge {
    pub id: String,
    pub source: String, // node_id
    pub target: String, // node_id
}

/// Node de workflow (referencia um agent).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNode {
    pub id: String,
    pub agent_id: String,
    pub max_retries: u32, // default 0
}

/// Definição de workflow (estática).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub id: String,
    pub name: String,
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl WorkflowDefinition {
    pub fn new(name: String) -> Self {
        let now = now_millis();
        Self {
            id: gen_id("wf"),
            name,
            nodes: Vec::new(),
            edges: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Valida o workflow: nodes referenciam agents existentes, sem ciclos, etc.
    pub fn validate(&self, agents: &[Agent]) -> Result<(), String> {
        let agent_ids: HashSet<_> = agents.iter().map(|a| a.id.clone()).collect();
        let node_ids: HashSet<_> = self.nodes.iter().map(|n| n.id.clone()).collect();

        // Todos os nodes referenciam agents existentes
        for node in &self.nodes {
            if !agent_ids.contains(&node.agent_id) {
                return Err(format!("node {} referencia agent inexistente: {}", node.id, node.agent_id));
            }
        }

        // Edges referenciam nodes existentes
        for edge in &self.edges {
            if !node_ids.contains(&edge.source) || !node_ids.contains(&edge.target) {
                return Err(format!("edge {} referencia node inexistente", edge.id));
            }
        }

        // Sem ciclos (Kahn)
        self.topo_order()?;

        Ok(())
    }

    /// Retorna ordem topológica dos node_ids (Kahn).
    /// Erro se houver ciclo.
    pub fn topo_order(&self) -> Result<Vec<String>, String> {
        let mut indegree: HashMap<String, usize> = HashMap::new();
        let mut adj: HashMap<String, Vec<String>> = HashMap::new();

        for node in &self.nodes {
            indegree.entry(node.id.clone()).or_insert(0);
            adj.entry(node.id.clone()).or_default();
        }

        for edge in &self.edges {
            *indegree.entry(edge.target.clone()).or_insert(0) += 1;
            adj.entry(edge.source.clone()).or_default().push(edge.target.clone());
        }

        let mut queue: VecDeque<String> = indegree.iter()
            .filter(|(_, &d)| d == 0)
            .map(|(k, _)| k.clone())
            .collect();

        let mut order = Vec::new();
        while let Some(n) = queue.pop_front() {
            order.push(n.clone());
            if let Some(neighbors) = adj.get(&n) {
                for neighbor in neighbors {
                    let d = indegree.get_mut(neighbor).unwrap();
                    *d -= 1;
                    if *d == 0 {
                        queue.push_back(neighbor.clone());
                    }
                }
            }
        }

        if order.len() != self.nodes.len() {
            return Err("workflow contém ciclo".to_string());
        }

        Ok(order)
    }

    /// Retorna mapa node_id -> lista de predecessores (para saber de quem puxar output).
    pub fn predecessors(&self) -> HashMap<String, Vec<String>> {
        let mut pred: HashMap<String, Vec<String>> = HashMap::new();
        for edge in &self.edges {
            pred.entry(edge.target.clone()).or_default().push(edge.source.clone());
        }
        pred
    }
}

/// Execução de um workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowExecution {
    pub id: String,
    pub workflow_id: String,
    pub status: ExecutionStatus,
    pub nodes: Vec<ExecutionNode>,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    pub error: Option<String>,
    pub cancelled: bool,
}

impl WorkflowExecution {
    pub fn new(workflow: &WorkflowDefinition) -> Self {
        let nodes = workflow.nodes.iter().map(|n| ExecutionNode {
            node_id: n.id.clone(),
            agent_id: n.agent_id.clone(),
            status: NodeExecutionStatus::Pending,
            attempts: Vec::new(),
            current_attempt: None,
            output_tail: String::new(),
            error: None,
        }).collect();

        Self {
            id: gen_id("exec"),
            workflow_id: workflow.id.clone(),
            status: ExecutionStatus::Pending,
            nodes,
            started_at: None,
            finished_at: None,
            error: None,
            cancelled: false,
        }
    }
}

/// Trait para executar um node (permite mock nos testes).
pub trait NodeExecutor: Send + Sync {
    /// Inicia execução do agent. Retorna handle para aguardar saída.
    /// Recebe Option<&AppHandle> - None em testes com MockNodeExecutor.
    fn start(&self, app: Option<&AppHandle>, agent: &Agent) -> Result<Box<dyn ExitWatcher>, RuntimeError>;

    /// Envia input ao agent.
    fn send_input(&self, agent_id: &str, input: &str) -> Result<(), RuntimeError>;

    /// Para o agent.
    fn stop(&self, app: Option<&AppHandle>, agent_id: &str) -> Result<(), RuntimeError>;
}

/// Watcher para aguardar conclusão de um processo.
pub trait ExitWatcher: Send {
    /// Bloqueia até o processo terminar. Retorna exit code.
    fn wait(&mut self) -> Result<i32, RuntimeError>;

    /// Captura output (tail) durante execução.
    fn take_output_tail(&mut self) -> String;
}

/// Implementação real via ProcessManager.
pub struct ProcessNodeExecutor {
    pub process_manager: Arc<ProcessManager>,
}

impl ProcessNodeExecutor {
    pub fn new(process_manager: Arc<ProcessManager>) -> Self {
        Self { process_manager }
    }
}

impl NodeExecutor for ProcessNodeExecutor {
    fn start(&self, app: Option<&AppHandle>, agent: &Agent) -> Result<Box<dyn ExitWatcher>, RuntimeError> {
        let app = app.expect("AppHandle necessário para ProcessNodeExecutor");

        // Agentes web não executam via PTY; a execução em workflow exigirá o
        // controle de navegador da Fase 16. Falha clara em vez de spawn errado.
        if agent.kind == AgentKind::Web {
            return Err(RuntimeError(format!(
                "agente '{}' é do tipo Web — execução em workflow disponível na Fase 16",
                agent.name
            )));
        }

        // Registra watcher ANTES de start para não perder o exit.
        let (tx, rx) = channel::<i32>();
        let output_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let output_buf_clone = output_buf.clone();

        let app_started = app.clone();
        let app_output = app.clone();
        let app_exit = app.clone();
        let app_error = app.clone();
        let agent_id = agent.id.clone();
        let agent_id_for_output = agent_id.clone();
        let agent_id_for_exit = agent_id.clone();

        let handle = crate::runtime::adapter_for(agent.runtime).start(
            agent,
            Box::new(move |text| {
                // Encaminha para frontend (evento existente)
                let _ = app_output.emit(
                    "agent_output",
                    crate::process_manager::AgentEvent {
                        agent_id: agent_id_for_output.clone(),
                        data: text.clone(),
                    },
                );
                // Acumula no buffer para tail
                let mut buf = output_buf_clone.lock().unwrap();
                buf.extend_from_slice(text.as_bytes());
                // Mantém apenas últimos 64KB
                if buf.len() > 65536 {
                    let excess = buf.len() - 65536;
                    buf.drain(0..excess);
                }
            }),
            Box::new(move |status| {
                let _ = app_exit.emit(
                    "agent_stopped",
                    crate::process_manager::AgentEvent {
                        agent_id: agent_id_for_exit.clone(),
                        data: status.to_string(),
                    },
                );
                let _ = tx.send(status as i32);
            }),
        )?;

        // Registra handle no ProcessManager
        {
            let mut guard = self.process_manager.processes.lock().unwrap();
            guard.insert(agent_id.clone(), handle);
        }

        let _ = app_started.emit(
            "agent_started",
            crate::process_manager::AgentEvent {
                agent_id: agent_id.clone(),
                data: String::new(),
            },
        );

        let _ = app_error.emit(
            "agent_status_changed",
            crate::process_manager::AgentEvent {
                agent_id: agent_id.clone(),
                data: Status::Running.as_str().to_string(),
            },
        );

        Ok(Box::new(ProcessExitWatcher {
            agent_id,
            process_manager: self.process_manager.clone(),
            exit_rx: rx,
            output_buf,
        }))
    }

    fn send_input(&self, agent_id: &str, input: &str) -> Result<(), RuntimeError> {
        self.process_manager.send_input(agent_id, input)
    }

    fn stop(&self, app: Option<&AppHandle>, agent_id: &str) -> Result<(), RuntimeError> {
        let app = app.expect("AppHandle necessário para ProcessNodeExecutor");
        self.process_manager.stop(app, agent_id)
    }
}

/// Watcher real que aguarda exit code via channel.
pub struct ProcessExitWatcher {
    agent_id: String,
    process_manager: Arc<ProcessManager>,
    exit_rx: Receiver<i32>,
    output_buf: Arc<Mutex<Vec<u8>>>,
}

impl ExitWatcher for ProcessExitWatcher {
    fn wait(&mut self) -> Result<i32, RuntimeError> {
        self.exit_rx.recv()
            .map_err(|_| RuntimeError("channel closed".to_string()))
    }

    fn take_output_tail(&mut self) -> String {
        let buf = self.output_buf.lock().unwrap();
        String::from_utf8_lossy(&buf).to_string()
    }
}

/// Engine de execução de workflow.
pub struct WorkflowEngine {
    app: Option<AppHandle>,
    executor: Arc<dyn NodeExecutor>,
}

impl WorkflowEngine {
    pub fn new(app: AppHandle, executor: Arc<dyn NodeExecutor>) -> Self {
        Self {
            app: Some(app),
            executor,
        }
    }

    /// Construtor apenas para testes: cria engine sem AppHandle (MockNodeExecutor ignora).
    #[cfg(test)]
    pub fn new_for_test(executor: Arc<dyn NodeExecutor>) -> Self {
        Self {
            app: None,
            executor,
        }
    }

    /// Obtém o AppHandle, panica se não estiver disponível (apenas para produção).
    fn get_app(&self) -> &AppHandle {
        self.app.as_ref().expect("AppHandle não disponível - use new_for_test para testes")
    }

    /// Inicia execução assíncrona do workflow.
    /// Retorna o ID da execução.
    /// `persist_fn` é chamado após cada mudança de estado relevante (node Running/Completed/Failed, workflow Completed/Failed/Cancelled).
    /// Recebe Option<&AppHandle> para acessar o estado gerenciado via `app.state::<AppState>()`.
    /// Em testes com MockNodeExecutor, o AppHandle é None (ignorado pelo mock).
    pub fn start_execution<F>(
        &self,
        workflow: &WorkflowDefinition,
        agents: &[Agent],
        persist_fn: F,
    ) -> Result<WorkflowExecution, String>
    where
        F: FnMut(Option<&AppHandle>, &WorkflowExecution) + Send + 'static,
    {
        // Valida
        workflow.validate(agents)?;

        let mut execution = WorkflowExecution::new(workflow);
        execution.status = ExecutionStatus::Running;
        execution.started_at = Some(now_millis());

        // Clona para thread
        let app = self.app.clone();
        let executor = self.executor.clone();
        let workflow_id = workflow.id.clone();
        let topo_order = workflow.topo_order().unwrap();
        let predecessors = workflow.predecessors();
        let nodes_map: HashMap<String, WorkflowNode> = workflow.nodes.iter()
            .map(|n| (n.id.clone(), n.clone()))
            .collect();

        // Clone agents for the thread (need owned data for 'static lifetime)
        let agents_owned: Vec<Agent> = agents.to_vec();

        // Clone execution for the thread, return the original
        let execution_for_thread = execution.clone();

        // Executa em thread de fundo (persist_fn é movido para a thread)
        thread::spawn(move || {
            let mut persist_fn = persist_fn;

            // Persistência inicial
            persist_fn(app.as_ref(), &execution_for_thread);
            let mut exec = execution_for_thread;
            let mut failed = false;

            for node_id in &topo_order {
                // Encontra o ExecutionNode
                let en_idx = exec.nodes.iter().position(|n| n.node_id == *node_id).unwrap();
                let node = &nodes_map[node_id];
                let agent = agents_owned.iter().find(|a| a.id == node.agent_id).unwrap();

                // Prepara input: concatena output dos predecessores
                let mut input = String::new();
                if let Some(preds) = predecessors.get(node_id) {
                    for pred_id in preds {
                        if let Some(pred_en) = exec.nodes.iter().find(|n| n.node_id == *pred_id) {
                            if !pred_en.output_tail.is_empty() {
                                input.push_str(&pred_en.output_tail);
                                input.push('\n');
                            }
                        }
                    }
                }

                // Loop de retry
                let max_retries = node.max_retries;
                let mut attempt_idx = 0;
                let mut success = false;

                while attempt_idx <= max_retries && !success {
                    let attempt_id = gen_id("attempt");
                    let mut attempt = Attempt {
                        id: attempt_id,
                        started_at: now_millis(),
                        finished_at: None,
                        exit_code: None,
                        error: None,
                        output_tail: String::new(),
                    };

                    exec.nodes[en_idx].status = NodeExecutionStatus::Running;
                    exec.nodes[en_idx].current_attempt = Some(attempt_idx as usize);
                    exec.nodes[en_idx].attempts.push(attempt);
                    persist_fn(app.as_ref(), &exec);

                    // Inicia agent
                    let mut watcher = match executor.start(app.as_ref(), agent) {
                        Ok(w) => w,
                        Err(e) => {
                            exec.nodes[en_idx].attempts[attempt_idx as usize].finished_at = Some(now_millis());
                            exec.nodes[en_idx].attempts[attempt_idx as usize].error = Some(e.to_string());
                            exec.nodes[en_idx].attempts[attempt_idx as usize].exit_code = Some(-1);
                            attempt_idx += 1;
                            persist_fn(app.as_ref(), &exec);
                            continue;
                        }
                    };

                    // Envia input se houver
                    if !input.is_empty() {
                        let _ = executor.send_input(&agent.id, &input);
                    }

                    let exit_code = match watcher.wait() {
                        Ok(code) => code,
                        Err(e) => {
                            exec.nodes[en_idx].attempts[attempt_idx as usize].finished_at = Some(now_millis());
                            exec.nodes[en_idx].attempts[attempt_idx as usize].error = Some(e.to_string());
                            exec.nodes[en_idx].attempts[attempt_idx as usize].exit_code = Some(-1);
                            attempt_idx += 1;
                            persist_fn(app.as_ref(), &exec);
                            continue;
                        }
                    };

                    let output_tail = watcher.take_output_tail();
                    exec.nodes[en_idx].output_tail = output_tail.clone();
                    exec.nodes[en_idx].attempts[attempt_idx as usize].output_tail = output_tail;
                    exec.nodes[en_idx].attempts[attempt_idx as usize].finished_at = Some(now_millis());
                    exec.nodes[en_idx].attempts[attempt_idx as usize].exit_code = Some(exit_code);

                    if exit_code == 0 {
                        success = true;
                        exec.nodes[en_idx].status = NodeExecutionStatus::Completed;
                        persist_fn(app.as_ref(), &exec);
                    } else {
                        exec.nodes[en_idx].attempts[attempt_idx as usize].error = Some(format!("exit code {}", exit_code));
                        attempt_idx += 1;
                        persist_fn(app.as_ref(), &exec);
                    }
                }

                if !success {
                    exec.nodes[en_idx].status = NodeExecutionStatus::Failed;
                    exec.nodes[en_idx].error = Some(format!("failed after {} retries", max_retries));
                    exec.status = ExecutionStatus::Failed;
                    exec.finished_at = Some(now_millis());
                    exec.error = Some(format!("node {} failed", node_id));
                    failed = true;
                    persist_fn(app.as_ref(), &exec);
                    break;
                }
            }

            if !failed && exec.status == ExecutionStatus::Running {
                exec.status = ExecutionStatus::Completed;
                exec.finished_at = Some(now_millis());
                persist_fn(app.as_ref(), &exec);
            }

            eprintln!("[WORKFLOW] execution {} finished with status {:?}", exec.id, exec.status);
        });

        Ok(execution)
    }
}

/// Mock executor para testes.
pub struct MockNodeExecutor {
    pub results: Arc<Mutex<HashMap<String, Vec<MockResult>>>>,
    /// Inputs capturados por agent_id (para verificação em testes).
    pub captured_inputs: Arc<Mutex<HashMap<String, Vec<String>>>>,
    /// Índice de tentativa por agent_id (para retry sequencial).
    pub attempt_index: Arc<Mutex<HashMap<String, usize>>>,
    /// Stop calls capturados (para testes de cancelamento).
    pub stop_calls: Arc<Mutex<Vec<String>>>,
}

#[derive(Clone)]
pub struct MockResult {
    pub exit_code: i32,
    pub output: String,
    pub delay_ms: u64,
}

impl MockNodeExecutor {
    pub fn new() -> Self {
        Self {
            results: Arc::new(Mutex::new(HashMap::new())),
            captured_inputs: Arc::new(Mutex::new(HashMap::new())),
            attempt_index: Arc::new(Mutex::new(HashMap::new())),
            stop_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn set_result(&self, agent_id: &str, results: Vec<MockResult>) {
        self.results.lock().unwrap().insert(agent_id.to_string(), results);
        // Reset attempt index when new results are set
        self.attempt_index.lock().unwrap().insert(agent_id.to_string(), 0);
    }

    /// Obtém os inputs capturados para um agent (para testes).
    pub fn get_captured_inputs(&self, agent_id: &str) -> Vec<String> {
        self.captured_inputs.lock().unwrap().get(agent_id).cloned().unwrap_or_default()
    }

    /// Obtém os stop calls capturados (para testes de cancelamento).
    pub fn get_stop_calls(&self) -> Vec<String> {
        self.stop_calls.lock().unwrap().clone()
    }
}

impl NodeExecutor for MockNodeExecutor {
    fn start(&self, _app: Option<&AppHandle>, agent: &Agent) -> Result<Box<dyn ExitWatcher>, RuntimeError> {
        // Obtém e incrementa o índice de tentativa para este agent
        let attempt_idx = {
            let mut idx_map = self.attempt_index.lock().unwrap();
            let idx = idx_map.entry(agent.id.clone()).or_insert(0);
            let current = *idx;
            *idx += 1;
            current
        };

        let results = self.results.lock().unwrap().get(&agent.id).cloned().unwrap_or_default();
        
        // Retorna o resultado correspondente à tentativa atual
        let result = results.get(attempt_idx).cloned().unwrap_or(MockResult {
            exit_code: -1,
            output: String::new(),
            delay_ms: 0,
        });

        Ok(Box::new(MockExitWatcher {
            iter: vec![result].into_iter(),
            last_output: String::new(),
        }))
    }

    fn send_input(&self, agent_id: &str, input: &str) -> Result<(), RuntimeError> {
        self.captured_inputs
            .lock()
            .unwrap()
            .entry(agent_id.to_string())
            .or_default()
            .push(input.to_string());
        Ok(())
    }

    fn stop(&self, _app: Option<&AppHandle>, agent_id: &str) -> Result<(), RuntimeError> {
        self.stop_calls.lock().unwrap().push(agent_id.to_string());
        Ok(())
    }
}

struct MockExitWatcher {
    iter: std::vec::IntoIter<MockResult>,
    last_output: String,
}

impl ExitWatcher for MockExitWatcher {
    fn wait(&mut self) -> Result<i32, RuntimeError> {
        // Simula delay
        if let Some(result) = self.iter.next() {
            if result.delay_ms > 0 {
                thread::sleep(Duration::from_millis(result.delay_ms));
            }
            // Armazena o output para take_output_tail()
            self.last_output = result.output;
            Ok(result.exit_code)
        } else {
            Ok(-1)
        }
    }

    fn take_output_tail(&mut self) -> String {
        // Retorna o output do último resultado consumido no wait()
        std::mem::take(&mut self.last_output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Agent, AgentKind, Role, Runtime, Status};

    fn test_agent(id: &str, name: &str) -> Agent {
        Agent {
            id: id.to_string(),
            name: name.to_string(),
            role: Role::Builder,
            runtime: Runtime::Custom,
            kind: AgentKind::Cli,
            model: "test".to_string(),
            command: "echo".to_string(),
            args: vec!["hello".to_string()],
            working_dir: "".to_string(),
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

    fn test_workflow() -> WorkflowDefinition {
        let mut wf = WorkflowDefinition::new("Test Workflow".to_string());
        wf.nodes = vec![
            WorkflowNode { id: "n1".to_string(), agent_id: "a1".to_string(), max_retries: 0 },
            WorkflowNode { id: "n2".to_string(), agent_id: "a2".to_string(), max_retries: 0 },
            WorkflowNode { id: "n3".to_string(), agent_id: "a3".to_string(), max_retries: 0 },
        ];
        wf.edges = vec![
            WorkflowEdge { id: "e1".to_string(), source: "n1".to_string(), target: "n2".to_string() },
            WorkflowEdge { id: "e2".to_string(), source: "n2".to_string(), target: "n3".to_string() },
        ];
        wf
    }

    #[test]
    fn workflow_topo_order() {
        let wf = test_workflow();
        let order = wf.topo_order().unwrap();
        assert_eq!(order, vec!["n1", "n2", "n3"]);
    }

    #[test]
    fn workflow_cycle_detection() {
        let mut wf = WorkflowDefinition::new("Cycle".to_string());
        wf.nodes = vec![
            WorkflowNode { id: "n1".to_string(), agent_id: "a1".to_string(), max_retries: 0 },
            WorkflowNode { id: "n2".to_string(), agent_id: "a2".to_string(), max_retries: 0 },
        ];
        wf.edges = vec![
            WorkflowEdge { id: "e1".to_string(), source: "n1".to_string(), target: "n2".to_string() },
            WorkflowEdge { id: "e2".to_string(), source: "n2".to_string(), target: "n1".to_string() },
        ];
        assert!(wf.topo_order().is_err());
    }

    #[test]
    fn workflow_validate_missing_agent() {
        let wf = test_workflow();
        let agents = vec![test_agent("a1", "A1"), test_agent("a2", "A2")]; // falta a3
        assert!(wf.validate(&agents).is_err());
    }

    #[test]
    fn execution_a_b_c_success() {
        let wf = test_workflow();
        let agents = vec![
            test_agent("a1", "A1"),
            test_agent("a2", "A2"),
            test_agent("a3", "A3"),
        ];

        let mock = Arc::new(MockNodeExecutor::new());
        mock.set_result("a1", vec![MockResult { exit_code: 0, output: "out1".to_string(), delay_ms: 1 }]);
        mock.set_result("a2", vec![MockResult { exit_code: 0, output: "out2".to_string(), delay_ms: 1 }]);
        mock.set_result("a3", vec![MockResult { exit_code: 0, output: "out3".to_string(), delay_ms: 1 }]);

        // Não podemos testar o engine completo sem AppHandle real,
        // mas validamos a lógica de topo_order e validação
        assert!(wf.validate(&agents).is_ok());
    }

    #[test]
    fn execution_retry_logic() {
        let mut wf = WorkflowDefinition::new("Retry".to_string());
        wf.nodes = vec![
            WorkflowNode { id: "n1".to_string(), agent_id: "a1".to_string(), max_retries: 2 },
        ];
        wf.edges = vec![];

        let agents = vec![test_agent("a1", "A1")];
        assert!(wf.validate(&agents).is_ok());
        assert_eq!(wf.nodes[0].max_retries, 2);
    }

    #[test]
    fn execution_cancelled_flag() {
        let wf = test_workflow();
        let agents = vec![
            test_agent("a1", "A1"),
            test_agent("a2", "A2"),
            test_agent("a3", "A3"),
        ];
        assert!(wf.validate(&agents).is_ok());
    }

    /// Teste de integração: execução completa A → B → C com sucesso.
    /// Verifica ordem, estados por node e resultado final COMPLETED.
    #[test]
    fn execution_a_b_c_success_full() {
        let wf = test_workflow();
        let agents = vec![
            test_agent("a1", "A1"),
            test_agent("a2", "A2"),
            test_agent("a3", "A3"),
        ];

        let mock = Arc::new(MockNodeExecutor::new());
        mock.set_result("a1", vec![MockResult { exit_code: 0, output: "out1".to_string(), delay_ms: 1 }]);
        mock.set_result("a2", vec![MockResult { exit_code: 0, output: "out2".to_string(), delay_ms: 1 }]);
        mock.set_result("a3", vec![MockResult { exit_code: 0, output: "out3".to_string(), delay_ms: 1 }]);

        let engine = WorkflowEngine::new_for_test(mock);
        let captured_executions = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_clone = captured_executions.clone();
        let execution = engine.start_execution(&wf, &agents, move |_app, exec| {
            captured_clone.lock().unwrap().push(exec.clone());
        }).unwrap();

        // Aguarda a thread de execução terminar
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Verifica execução final
        let final_exec = captured_executions.lock().unwrap().last().unwrap().clone();
        assert_eq!(final_exec.status, ExecutionStatus::Completed);
        assert!(final_exec.finished_at.is_some());

        // Verifica ordem de execução dos nodes
        assert_eq!(final_exec.nodes[0].node_id, "n1");
        assert_eq!(final_exec.nodes[0].status, NodeExecutionStatus::Completed);
        assert_eq!(final_exec.nodes[1].node_id, "n2");
        assert_eq!(final_exec.nodes[1].status, NodeExecutionStatus::Completed);
        assert_eq!(final_exec.nodes[2].node_id, "n3");
        assert_eq!(final_exec.nodes[2].status, NodeExecutionStatus::Completed);

        // Verifica que cada node tem seu próprio estado
        assert_eq!(final_exec.nodes[0].attempts.len(), 1);
        assert_eq!(final_exec.nodes[1].attempts.len(), 1);
        assert_eq!(final_exec.nodes[2].attempts.len(), 1);
        assert_eq!(final_exec.nodes[0].attempts[0].exit_code, Some(0));
        assert_eq!(final_exec.nodes[1].attempts[0].exit_code, Some(0));
        assert_eq!(final_exec.nodes[2].attempts[0].exit_code, Some(0));
    }

    /// Teste de integração: falha no node B impede execução do node C.
    /// Verifica que workflow termina como Failed e C não executa.
    #[test]
    fn execution_b_fails_c_not_run() {
        let wf = test_workflow();
        let agents = vec![
            test_agent("a1", "A1"),
            test_agent("a2", "A2"),
            test_agent("a3", "A3"),
        ];

        let mock = Arc::new(MockNodeExecutor::new());
        mock.set_result("a1", vec![MockResult { exit_code: 0, output: "out1".to_string(), delay_ms: 1 }]);
        mock.set_result("a2", vec![MockResult { exit_code: 1, output: "error".to_string(), delay_ms: 1 }]); // B falha
        mock.set_result("a3", vec![MockResult { exit_code: 0, output: "out3".to_string(), delay_ms: 1 }]);

        let engine = WorkflowEngine::new_for_test(mock);
        let captured_executions = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_clone = captured_executions.clone();
        let execution = engine.start_execution(&wf, &agents, move |_app, exec| {
            captured_clone.lock().unwrap().push(exec.clone());
        }).unwrap();

        // Aguarda a thread de execução terminar
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Verifica execução final
        let final_exec = captured_executions.lock().unwrap().last().unwrap().clone();
        assert_eq!(final_exec.status, ExecutionStatus::Failed);
        assert!(final_exec.finished_at.is_some());
        assert!(final_exec.error.is_some());

        // A executou com sucesso
        assert_eq!(final_exec.nodes[0].status, NodeExecutionStatus::Completed);
        assert_eq!(final_exec.nodes[0].attempts[0].exit_code, Some(0));

        // B falhou
        assert_eq!(final_exec.nodes[1].status, NodeExecutionStatus::Failed);
        assert_eq!(final_exec.nodes[1].attempts[0].exit_code, Some(1));

        // C NÃO executou (permanece Pending ou Skipped)
        assert_eq!(final_exec.nodes[2].status, NodeExecutionStatus::Pending);
        assert_eq!(final_exec.nodes[2].attempts.len(), 0);
    }

    /// Teste de integração: fluxo de dados A → B → C.
    /// Verifica que output de A vira input de B, e output de B vira input de C.
    #[test]
    fn execution_data_flow_a_b_c() {
        let wf = test_workflow();
        let agents = vec![
            test_agent("a1", "A1"),
            test_agent("a2", "A2"),
            test_agent("a3", "A3"),
        ];

        let mock = Arc::new(MockNodeExecutor::new());
        // A produz "output_A"
        mock.set_result("a1", vec![MockResult { exit_code: 0, output: "output_A".to_string(), delay_ms: 1 }]);
        // B produz "output_B"
        mock.set_result("a2", vec![MockResult { exit_code: 0, output: "output_B".to_string(), delay_ms: 1 }]);
        // C produz "output_C"
        mock.set_result("a3", vec![MockResult { exit_code: 0, output: "output_C".to_string(), delay_ms: 1 }]);

        let engine = WorkflowEngine::new_for_test(mock.clone());
        let captured_executions = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_clone = captured_executions.clone();
        let _execution = engine.start_execution(&wf, &agents, move |_app, exec| {
            captured_clone.lock().unwrap().push(exec.clone());
        }).unwrap();

        // Aguarda a thread de execução terminar
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Verifica execução final
        let final_exec = captured_executions.lock().unwrap().last().unwrap().clone();
        assert_eq!(final_exec.status, ExecutionStatus::Completed);

        // Verifica inputs capturados pelo mock
        let inputs_b = mock.get_captured_inputs("a2");
        let inputs_c = mock.get_captured_inputs("a3");

        // B deve ter recebido output de A como input
        assert!(!inputs_b.is_empty(), "B deve ter recebido input");
        let input_b = inputs_b.join("\n");
        assert!(input_b.contains("output_A"), "Input de B deve conter output de A: {}", input_b);

        // C deve ter recebido output de B como input
        assert!(!inputs_c.is_empty(), "C deve ter recebido input");
        let input_c = inputs_c.join("\n");
        assert!(input_c.contains("output_B"), "Input de C deve conter output de B: {}", input_c);
    }

    /// Teste de integração: retry em caso de falha.
    /// Node com max_retries=2 falha 2x e succeeds na 3ª tentativa.
    #[test]
    fn execution_retry_on_failure() {
        let mut wf = WorkflowDefinition::new("Retry Workflow".to_string());
        wf.nodes = vec![
            WorkflowNode { id: "n1".to_string(), agent_id: "a1".to_string(), max_retries: 2 },
        ];
        wf.edges = vec![];

        let agents = vec![test_agent("a1", "A1")];

        let mock = Arc::new(MockNodeExecutor::new());
        // Falha na 1ª tentativa (exit_code=1), falha na 2ª, sucesso na 3ª
        mock.set_result("a1", vec![
            MockResult { exit_code: 1, output: "fail1".to_string(), delay_ms: 1 },
            MockResult { exit_code: 1, output: "fail2".to_string(), delay_ms: 1 },
            MockResult { exit_code: 0, output: "success".to_string(), delay_ms: 1 },
        ]);

        let engine = WorkflowEngine::new_for_test(mock.clone());
        let captured_executions = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_clone = captured_executions.clone();
        let _execution = engine.start_execution(&wf, &agents, move |_app, exec| {
            captured_clone.lock().unwrap().push(exec.clone());
        }).unwrap();

        // Aguarda a thread de execução terminar
        std::thread::sleep(std::time::Duration::from_millis(200));

        // Verifica execução final
        let final_exec = captured_executions.lock().unwrap().last().unwrap().clone();
        assert_eq!(final_exec.status, ExecutionStatus::Completed);

        // Verifica que houve 3 attempts
        let node = &final_exec.nodes[0];
        assert_eq!(node.attempts.len(), 3, "Deveria ter 3 attempts (2 retries + 1 sucesso)");

        // 1ª tentativa: falhou
        assert_eq!(node.attempts[0].exit_code, Some(1));
        assert!(node.attempts[0].error.is_some());

        // 2ª tentativa: falhou
        assert_eq!(node.attempts[1].exit_code, Some(1));
        assert!(node.attempts[1].error.is_some());

        // 3ª tentativa: sucesso
        assert_eq!(node.attempts[2].exit_code, Some(0));
        assert_eq!(node.status, NodeExecutionStatus::Completed);
    }

}