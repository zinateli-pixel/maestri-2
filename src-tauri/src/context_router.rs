//! Roteamento de contexto entre agentes (Fase 3).
//!
//! Responsabilidades separadas:
//! - `Agent` (models.rs): entidade lógica do Maestro.
//! - `Edge` (models.rs): relação A -> B e seu tipo — diz O QUE está ligado.
//! - Este módulo: transporte/roteamento — diz COMO entregar uma mensagem de
//!   contexto ao destino.
//!
//! O formato de mensagem é mínimo e interno (`RoutedMessage` + `MessageKind`),
//! preparado para evoluir (hoje só `Context`; futuramente TASK/MESSAGE/RESULT).
//!
//! Nota de design: NÃO encaminhamos automaticamente o stdout cru do PTY.
//! O output bruto não tem limites de mensagem e auto-injetá-lo no stdin do
//! destino poderia causar loops de eco (edges recíprocas) e até executar
//! comandos arbitrários quando o destino é um shell. O encaminhamento é
//! EXPLÍCITO, via o comando `route_context` (ou futuramente pelo protocolo/
//! workflow engine).

use crate::models::{Agent, AgentKind, Edge};
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

/// Tipo de mensagem de contexto. Hoje apenas `Context`; extensível a
/// TASK/MESSAGE/RESULT nas próximas fases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Context,
}

/// Mensagem interna roteada entre um agente origem e um destino.
/// Autodescritiva: identidade, workspace, origem, destino, tipo, payload e
/// timestamp. É o formato do protocolo de comunicação entre agentes,
/// independente do transporte (CLI/Web/App).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutedMessage {
    /// Identidade única da mensagem (usada para bloquear duplicação).
    pub id: String,
    /// Workspace/canvas ao qual a mensagem pertence (base do isolamento).
    pub workspace_id: String,
    /// Origem (id do agente remetente).
    pub source: String,
    /// Destino (id do agente destinatário).
    pub target: String,
    /// Tipo de mensagem.
    pub kind: MessageKind,
    /// Conteúdo.
    pub payload: String,
    /// Timestamp (ms) de criação.
    pub timestamp: u64,
    /// Referência mínima opcional a uma skill (Fase 9). Sem execução ainda.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_id: Option<String>,
    /// Referência mínima opcional a uma memória (Fase 11). Sem RAG ainda.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_id: Option<String>,
}

impl RoutedMessage {
    pub fn new(
        workspace_id: &str,
        source: &str,
        target: &str,
        kind: MessageKind,
        payload: &str,
        timestamp: u64,
    ) -> Self {
        Self {
            id: generate_message_id(timestamp),
            workspace_id: workspace_id.to_string(),
            source: source.to_string(),
            target: target.to_string(),
            kind,
            payload: payload.to_string(),
            timestamp,
            skill_id: None,
            memory_id: None,
        }
    }

    /// Anexa a referência mínima a uma skill à mensagem.
    pub fn with_skill(mut self, skill_id: impl Into<String>) -> Self {
        self.skill_id = Some(skill_id.into());
        self
    }

    /// Anexa a referência mínima a uma memória à mensagem.
    pub fn with_memory(mut self, memory_id: impl Into<String>) -> Self {
        self.memory_id = Some(memory_id.into());
        self
    }

    /// Envelope explícito injetado no stdin do destino. Deixa claro que é
    /// contexto roteado (não digitação do usuário) e termina em newline.
    pub fn envelope(&self) -> String {
        format!("[context {} -> {}] {}\n", self.source, self.target, self.payload)
    }
}

/// Resultado da entrega de uma mensagem a um destino.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryReport {
    pub target: String,
    pub delivered: bool,
    pub error: Option<String>,
}

/// Trait de transporte: entrega uma mensagem de contexto ao agente destino.
/// Isolada por runtime — hoje só há CLI (injeção no stdin via PTY); Web/App
/// terão outras implementações sem tocar no router.
///
/// O protocolo (RoutedMessage) é independente do transporte: a mensagem já
/// carrega seu próprio `workspace_id`; o transporte apenas a respeita.
pub trait ContextTransport: Send + Sync {
    fn deliver(
        &self,
        app: Option<&AppHandle>,
        target: &Agent,
        message: &RoutedMessage,
    ) -> Result<(), String>;
}

/// Transporte CLI atual: injeta o contexto no stdin do processo do destino
/// (somente se estiver em execução NO MESMO workspace). É a adaptação
/// específica de runtime.
pub struct CliContextTransport;

impl ContextTransport for CliContextTransport {
    fn deliver(
        &self,
        app: Option<&AppHandle>,
        target: &Agent,
        message: &RoutedMessage,
    ) -> Result<(), String> {
        if target.kind != AgentKind::Cli {
            return Err(format!("destino {} não é um agente CLI", target.id));
        }
        let app = app.ok_or_else(|| "AppHandle indisponível".to_string())?;
        let state = app.state::<AppState>();

        // Bloqueia duplicação: mesma identidade de mensagem só é entregue uma vez.
        if state.was_delivered(&message.id) {
            return Err("mensagem duplicada (id já entregue)".to_string());
        }

        // Valida workspace: o destino precisa rodar no workspace da mensagem.
        if !state.processes.is_running_in(&message.workspace_id, &target.id) {
            return Err(format!("agente destino não está em execução: {}", target.id));
        }

        // Envelope explícito deixa claro que é contexto roteado, não digitação do usuário.
        let envelope = message.envelope();
        state
            .processes
            .send_input_in(&message.workspace_id, &target.id, &envelope)
            .map_err(|e| e.to_string())?;

        // Marca como entregue somente após sucesso (retry futuro não é bloqueado).
        state.mark_delivered(&message.id);
        Ok(())
    }
}

/// Router de contexto: resolve destinos a partir das edges e delega a entrega
/// ao transporte. Não conhece detalhes de runtime.
#[derive(Clone)]
pub struct ContextRouter {
    transport: Arc<dyn ContextTransport>,
}

impl ContextRouter {
    pub fn new(transport: Arc<dyn ContextTransport>) -> Self {
        Self { transport }
    }

    /// Retorna os ids de destino conectados a partir de `source_id`.
    pub fn outgoing_targets(edges: &[Edge], source_id: &str) -> Vec<String> {
        edges
            .iter()
            .filter(|e| e.source == source_id)
            .map(|e| e.target.clone())
            .collect()
    }

    /// Roteia `payload` da origem para todos os destinos conectados.
    pub fn route(
        &self,
        app: Option<&AppHandle>,
        workspace_id: &str,
        agents: &[Agent],
        edges: &[Edge],
        source_id: &str,
        payload: &str,
    ) -> Vec<DeliveryReport> {
        let timestamp = now_ms();
        let mut reports = Vec::new();

        for target_id in Self::outgoing_targets(edges, source_id) {
            let target = agents.iter().find(|a| a.id == target_id);
            let Some(target) = target else {
                reports.push(DeliveryReport {
                    target: target_id,
                    delivered: false,
                    error: Some("destino inexistente".to_string()),
                });
                continue;
            };

            let message = RoutedMessage::new(workspace_id, source_id, &target_id, MessageKind::Context, payload, timestamp);
            let result = self.transport.deliver(app, target, &message);
            reports.push(DeliveryReport {
                target: target_id,
                delivered: result.is_ok(),
                error: result.err(),
            });
        }

        reports
    }

    /// Roteia `payload` da origem para UM destino específico, resolvido por
    /// nome OU id e restrito aos peers conectados à origem (topologia das
    /// edges). Usado quando o próprio agente inicia o envio (Fase 4) — nunca
    /// encaminha para um agente com o qual não esteja ligado.
    pub fn route_to_peer(
        &self,
        app: Option<&AppHandle>,
        workspace_id: &str,
        agents: &[Agent],
        edges: &[Edge],
        source_id: &str,
        target_ref: &str,
        payload: &str,
    ) -> DeliveryReport {
        // Bloqueia ciclo básico: auto-mensagem (origem == destino).
        if source_id == target_ref {
            return DeliveryReport {
                target: target_ref.to_string(),
                delivered: false,
                error: Some("não é possível enviar uma mensagem para si mesmo".to_string()),
            };
        }

        let Some(target_id) = resolve_peer_id(agents, edges, source_id, target_ref) else {
            return DeliveryReport {
                target: target_ref.to_string(),
                delivered: false,
                error: Some("destino não conectado/encontrado".to_string()),
            };
        };
        let Some(target) = agents.iter().find(|a| a.id == target_id) else {
            return DeliveryReport {
                target: target_id,
                delivered: false,
                error: Some("destino inexistente".to_string()),
            };
        };

        let message = RoutedMessage::new(
            workspace_id,
            source_id,
            &target_id,
            MessageKind::Context,
            payload,
            now_ms(),
        );
        let result = self.transport.deliver(app, target, &message);
        DeliveryReport {
            target: target_id,
            delivered: result.is_ok(),
            error: result.err(),
        }
    }

    /// Entrega uma mensagem a um destino específico (já resolvido por id) SEM
    /// exigir conectividade por edge. Usado pelo Workflow Engine (Fase 6)
    /// quando a origem é o próprio disparador (não um agente do canvas).
    /// O transporte continua validando workspace, identidade e duplicação.
    pub fn deliver_direct(
        &self,
        app: Option<&AppHandle>,
        workspace_id: &str,
        agents: &[Agent],
        source: &str,
        target_id: &str,
        kind: MessageKind,
        payload: &str,
    ) -> DeliveryReport {
        let Some(target) = agents.iter().find(|a| a.id == target_id) else {
            return DeliveryReport {
                target: target_id.to_string(),
                delivered: false,
                error: Some("destino inexistente".to_string()),
            };
        };
        let message = RoutedMessage::new(workspace_id, source, target_id, kind, payload, now_ms());
        let result = self.transport.deliver(app, target, &message);
        DeliveryReport {
            target: target_id.to_string(),
            delivered: result.is_ok(),
            error: result.err(),
        }
    }
}

/// Encaminha explicitamente um payload da origem para os destinos conectados.
#[tauri::command]
pub fn route_context(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: String,
    payload: String,
) -> Result<Vec<DeliveryReport>, String> {
    Ok(state.route_context_from(&app, &source_id, &payload))
}

/// Encaminha um payload da origem para UM peer específico (nome ou id),
/// resolvido pela topologia das edges. Usado pelo envio iniciado pelo agente.
#[tauri::command]
pub fn route_context_to(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: String,
    target: String,
    payload: String,
) -> Result<DeliveryReport, String> {
    Ok(state.route_context_to_peer(&app, &source_id, &target, &payload))
}

// ---------------------------------------------------------------------------
// Descoberta e protocolo de envio iniciado pelo agente (Fase 4).
// Um agente CLI não tem acesso a filesystem/processos/internet, então o
// Maestro (a) anuncia quem são seus peers conectados e (b) interpreta uma
// linha de protocolo no output para rotear a mensagem ao peer correto.
// ---------------------------------------------------------------------------

/// Prefixo de qualquer linha de protocolo do Maestro.
pub const PROTOCOL_PREFIX: &str = "[[MESTRO:";
/// Prefixo da diretiva de envio: `[[MESTRO:send <alvo>]] <payload>`.
pub const SEND_PREFIX: &str = "[[MESTRO:send ";
/// Diretiva para (re)listar os peers conectados: `[[MESTRO:peers]]`.
pub const PEERS_DIRECTIVE: &str = "[[MESTRO:peers]]";

/// Diretiva de protocolo extraída da saída de um agente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolDirective {
    /// Enviar contexto/mensagem para um peer conectado.
    Send { target: String, payload: String },
    /// Solicitar a lista de peers conectados.
    Peers,
}

/// Resultado de um passo do scanner.
#[derive(Debug, Default)]
pub struct ScanResult {
    /// Texto normal a encaminhar ao terminal/barramento.
    pub forward: String,
    /// Diretivas de protocolo detectadas (são tratadas, não encaminhadas).
    pub directives: Vec<ProtocolDirective>,
}

/// Scanner incremental das linhas de protocolo na saída do agente.
///
/// Mantém apenas a "cauda" de uma linha que começa com `PROTOCOL_PREFIX`
/// (potencial diretiva). Qualquer outra saída — inclusive spinners/ANSI sem
/// newline — é encaminhada imediatamente, sem atraso perceptível.
pub struct ProtocolScanner {
    pending: String,
}

impl ProtocolScanner {
    /// Tamanho máximo da cauda retida. Evita memória crescente se um agente
    /// emitir uma linha gigante iniciada por `[[MESTRO:` sem newline.
    const MAX_PENDING: usize = 4096;

    pub fn new() -> Self {
        Self {
            pending: String::new(),
        }
    }

    /// Consome um chunk, separando texto normal de diretivas de envio.
    pub fn feed(&mut self, chunk: &str) -> ScanResult {
        let mut result = ScanResult::default();
        self.pending.push_str(chunk);

        loop {
            match self.pending.find('\n') {
                Some(pos) => {
                    let line: String = self.pending.drain(..=pos).collect();
                    match parse_protocol_line(&line) {
                        Some(d) => result.directives.push(d),
                        None => result.forward.push_str(&line),
                    }
                }
                None => break,
            }
        }

        // Cauda sem newline: mantém apenas se puder virar (início de) uma
        // diretiva; caso contrário, encaminha imediatamente (sem atraso).
        if !self.pending.is_empty()
            && (!tail_could_contain_directive(&self.pending)
                || self.pending.len() > Self::MAX_PENDING)
        {
            result.forward.push_str(&self.pending);
            self.pending.clear();
        }

        result
    }
}

/// Indica se a cauda (sem newline) ainda pode se tornar uma diretiva:
/// contém o prefixo do protocolo ou termina com um prefixo dele (split de
/// chunk). Remove ANSI antes de inspecionar.
fn tail_could_contain_directive(tail: &str) -> bool {
    let clean = strip_ansi(tail);
    if clean.contains(PROTOCOL_PREFIX) {
        return true;
    }
    let max = PROTOCOL_PREFIX.len().min(clean.len());
    for end in 1..=max {
        if clean.ends_with(&PROTOCOL_PREFIX[..end]) {
            return true;
        }
    }
    false
}

/// Remove sequências de escape ANSI (CSI) de uma string, preservando UTF-8.
fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            i += 1;
            if i < bytes.len() && bytes[i] == b'[' {
                i += 1;
                while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            }
        } else {
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Interpreta uma linha completa como diretiva de protocolo, se aplicável.
/// Remove ANSI e whitespace inicial; a diretiva precisa estar no início da
/// linha (evita falsos positivos quando o agente ecoa as instruções do banner).
fn parse_protocol_line(line: &str) -> Option<ProtocolDirective> {
    let clean = strip_ansi(line);
    let trimmed = clean.trim();

    // Peers: exige a linha inteira `[[MESTRO:peers]]`.
    if let Some(rest) = trimmed.strip_prefix(PEERS_DIRECTIVE) {
        if rest.trim().is_empty() {
            return Some(ProtocolDirective::Peers);
        }
        return None;
    }

    let rest = trimmed.strip_prefix(SEND_PREFIX)?;
    let close = rest.find("]]")?;
    let target = rest[..close].trim();
    if target.is_empty() {
        return None;
    }
    let payload = rest[close + 2..].trim();
    if payload.is_empty() {
        return None;
    }
    Some(ProtocolDirective::Send {
        target: target.to_string(),
        payload: payload.to_string(),
    })
}

/// Retorna os ids dos agentes conectados a `agent_id` por qualquer edge
/// (origem OU destino). No canvas, "conectado" é uma relação não-direcional
/// para comunicação: uma edge A—B permite A falar com B e B falar com A.
pub fn connected_peer_ids(edges: &[Edge], agent_id: &str) -> Vec<String> {
    let mut peers: Vec<String> = Vec::new();
    for e in edges {
        if e.source == agent_id {
            peers.push(e.target.clone());
        } else if e.target == agent_id {
            peers.push(e.source.clone());
        }
    }
    peers.sort();
    peers.dedup();
    peers
}

/// Resolve o id do peer de destino a partir de nome OU id, restrito aos
/// peers efetivamente conectados à origem. Garante que um agente só consiga
/// endereçar outro com o qual esteja ligado — em qualquer direção da edge.
pub fn resolve_peer_id(
    agents: &[Agent],
    edges: &[Edge],
    source_id: &str,
    target_ref: &str,
) -> Option<String> {
    let target_ref_lower = target_ref.to_lowercase();
    for target_id in connected_peer_ids(edges, source_id) {
        if let Some(agent) = agents.iter().find(|a| a.id == target_id) {
            if agent.id == target_ref || agent.name.to_lowercase() == target_ref_lower {
                return Some(target_id);
            }
        }
    }
    None
}

/// Cabeçalho de identidade injetado em TODO agente CLI ao iniciar, mesmo sem
/// peers conectados. Deixa explícito que o processo roda DENTRO do MAESTRO 2.0,
/// que o canvas é a única fonte de verdade e que NÃO deve depender do Maestri
/// original, `maestri list`, `maestri ask`, OmniRoute nem de descoberta externa.
const IDENTITY_HEADER: &str = "\
[MESTRO] You are running inside MAESTRO 2.0, a visual multi-agent orchestration platform. The canvas is the single source of truth for this environment.\n\
[MESTRO] Do NOT use `maestri list`, `maestri ask`, OmniRoute, or any filesystem/process/network lookup to discover other agents. Those tools are not part of MAESTRO 2.0.\n\
[MESTRO] Communicate only through the internal protocol directives below (they are MAESTRO 2.0 directives, not shell commands).\n";

/// Diretivas de protocolo reafirmadas no banner. Escritas de modo que a
/// diretiva nunca apareça no INÍCIO de uma linha (evita que o scanner
/// interprete o eco do banner como uma diretiva real de send/list).
const DIRECTIVE_HINTS: &str = "\
[MESTRO] To re-list your connected peers, print the directive [[MESTRO:peers]] on its own line.\n\
[MESTRO] To send a message to a connected peer, print the directive [[MESTRO:send <name_or_id>]] <your message> on its own line.\n";

/// Constrói a mensagem de descoberta (banner/notícia de identidade) para o
/// agente `source_id`. SEMPRE retorna conteúdo: identidade do MAESTRO 2.0 +
/// diretivas do protocolo + a lista de peers conectados (ou "none"). Redigida
/// como instrução imperativa para o modelo seguir, impedindo que ele procure
/// peers no filesystem/processos/rede ou em CLIs externas como o Maestri.
pub fn build_peers_banner(agents: &[Agent], edges: &[Edge], source_id: &str) -> String {
    let mut peers = Vec::new();
    for target_id in connected_peer_ids(edges, source_id) {
        if let Some(a) = agents.iter().find(|x| x.id == target_id) {
            peers.push(format!("{} (id={})", a.name, a.id));
        }
    }

    let peers_line = if peers.is_empty() {
        "[MESTRO] Connected peers: none.\n".to_string()
    } else {
        format!("[MESTRO] Connected peers: {}.\n", peers.join("; "))
    };

    format!("{}{}{}", IDENTITY_HEADER, DIRECTIVE_HINTS, peers_line)
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Contador local para compor identidades de mensagem únicas por sessão.
static MSG_SEQ: AtomicU64 = AtomicU64::new(0);

/// Gera uma identidade única de mensagem: `msg-<timestamp>-<seq>`.
fn generate_message_id(timestamp: u64) -> String {
    let seq = MSG_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("msg-{}-{}", timestamp, seq)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Agent, Edge, EdgeType, Role, Runtime, Status};
    use std::collections::HashSet;
    use std::sync::Mutex;

    fn cli_agent(id: &str) -> Agent {
        Agent {
            id: id.to_string(),
            name: id.to_string(),
            role: Role::Observer,
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

    fn sample_edge(source: &str, target: &str) -> Edge {
        Edge {
            id: format!("{}-{}", source, target),
            source: source.to_string(),
            target: target.to_string(),
            source_handle: None,
            target_handle: None,
            edge_type: EdgeType::Message,
            label: None,
        }
    }

    /// Transporte fake: registra as entregas e simula destinos não rodando.
    struct MockTransport {
        delivered: Mutex<Vec<(String, String)>>, // (target_id, payload)
        running: HashSet<String>,
        seen_workspaces: Mutex<Vec<String>>,
        /// Se definido, só aceita entregas neste workspace (simula isolamento).
        workspace: Option<String>,
    }

    impl MockTransport {
        fn new(running: &[&str]) -> Self {
            Self {
                delivered: Mutex::new(Vec::new()),
                running: running.iter().map(|s| s.to_string()).collect(),
                seen_workspaces: Mutex::new(Vec::new()),
                workspace: None,
            }
        }

        fn with_workspace(mut self, ws: &str) -> Self {
            self.workspace = Some(ws.to_string());
            self
        }

        fn delivered_targets(&self) -> Vec<String> {
            self.delivered
                .lock()
                .unwrap()
                .iter()
                .map(|(t, _)| t.clone())
                .collect()
        }

        fn delivered_payloads(&self) -> Vec<String> {
            self.delivered
                .lock()
                .unwrap()
                .iter()
                .map(|(_, p)| p.clone())
                .collect()
        }

        fn delivered_pairs(&self) -> Vec<(String, String)> {
            self.delivered.lock().unwrap().clone()
        }

        fn seen_workspaces(&self) -> Vec<String> {
            self.seen_workspaces.lock().unwrap().clone()
        }
    }

    impl ContextTransport for MockTransport {
        fn deliver(
            &self,
            _app: Option<&AppHandle>,
            target: &Agent,
            message: &RoutedMessage,
        ) -> Result<(), String> {
            let workspace_id = message.workspace_id.clone();
            self.seen_workspaces
                .lock()
                .unwrap()
                .push(workspace_id.clone());

            // Fronteira de isolamento: este agente só existe no workspace X.
            if let Some(expected) = &self.workspace {
                if expected != &workspace_id {
                    return Err("destino pertence a outro workspace".to_string());
                }
            }

            if !self.running.contains(&target.id) {
                return Err(format!("agente destino não está em execução: {}", target.id));
            }
            self.delivered
                .lock()
                .unwrap()
                .push((target.id.clone(), message.payload.clone()));
            Ok(())
        }
    }

    #[test]
    fn route_delivers_to_connected_running_target() {
        let a = cli_agent("a1");
        let b = cli_agent("a2");
        let agents = vec![a, b];
        let edges = vec![sample_edge("a1", "a2")];
        let transport = Arc::new(MockTransport::new(&["a2"]));
        let router = ContextRouter::new(transport.clone());

        let reports = router.route(None, "ws1", &agents, &edges, "a1", "alô");

        assert_eq!(reports.len(), 1);
        assert!(reports[0].delivered);
        assert_eq!(reports[0].target, "a2");
        assert_eq!(transport.delivered_targets(), vec!["a2".to_string()]);
    }

    #[test]
    fn route_skips_nonexistent_target() {
        let a = cli_agent("a1");
        let agents = vec![a];
        let edges = vec![sample_edge("a1", "ghost")];
        let transport = Arc::new(MockTransport::new(&[]));
        let router = ContextRouter::new(transport);

        let reports = router.route(None, "ws1", &agents, &edges, "a1", "x");

        assert_eq!(reports.len(), 1);
        assert!(!reports[0].delivered);
        let err = reports[0].error.as_deref().unwrap();
        assert!(err.contains("inexistente"));
    }

    #[test]
    fn route_reports_target_not_running() {
        let a = cli_agent("a1");
        let b = cli_agent("a2");
        let agents = vec![a, b];
        let edges = vec![sample_edge("a1", "a2")];
        let transport = Arc::new(MockTransport::new(&[])); // a2 não está rodando
        let router = ContextRouter::new(transport);

        let reports = router.route(None, "ws1", &agents, &edges, "a1", "x");

        assert_eq!(reports.len(), 1);
        assert!(!reports[0].delivered);
        let err = reports[0].error.as_deref().unwrap();
        assert!(err.contains("não está em execução"));
    }

    #[test]
    fn route_with_no_edges_returns_empty() {
        let a = cli_agent("a1");
        let agents = vec![a];
        let edges: Vec<Edge> = vec![];
        let transport = Arc::new(MockTransport::new(&[]));
        let router = ContextRouter::new(transport);

        let reports = router.route(None, "ws1", &agents, &edges, "a1", "x");

        assert!(reports.is_empty());
    }

    #[test]
    fn outgoing_targets_respects_direction() {
        let edges = vec![sample_edge("a1", "a2"), sample_edge("a2", "a1")];
        assert_eq!(ContextRouter::outgoing_targets(&edges, "a1"), vec!["a2".to_string()]);
        assert_eq!(ContextRouter::outgoing_targets(&edges, "a2"), vec!["a1".to_string()]);
    }

    #[test]
    fn envelope_is_explicit_and_newline_terminated() {
        let m = RoutedMessage::new("ws1", "a1", "a2", MessageKind::Context, "alô", 123);
        assert_eq!(m.envelope(), "[context a1 -> a2] alô\n");
    }

    #[test]
    fn message_carries_full_identity_and_workspace() {
        let m = RoutedMessage::new("ws1", "a1", "a2", MessageKind::Context, "x", 123);

        assert_eq!(m.workspace_id, "ws1");
        assert_eq!(m.source, "a1");
        assert_eq!(m.target, "a2");
        assert_eq!(m.kind, MessageKind::Context);
        assert_eq!(m.payload, "x");
        assert_eq!(m.timestamp, 123);
        assert!(!m.id.is_empty());

        // Identidade única entre mensagens.
        let m2 = RoutedMessage::new("ws1", "a1", "a2", MessageKind::Context, "x", 123);
        assert_ne!(m.id, m2.id);
    }

    #[test]
    fn message_can_carry_skill_reference() {
        let m = RoutedMessage::new("ws1", "a1", "a2", MessageKind::Context, "x", 123)
            .with_skill("s1");
        assert_eq!(m.skill_id.as_deref(), Some("s1"));

        let json = serde_json::to_string(&m).unwrap();
        let back: RoutedMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.skill_id.as_deref(), Some("s1"));

        // Sem skill: campo omitido no JSON e desserializa como None.
        let plain = RoutedMessage::new("ws1", "a1", "a2", MessageKind::Context, "x", 123);
        let plain_json = serde_json::to_string(&plain).unwrap();
        assert!(!plain_json.contains("skill_id"));
        assert_eq!(
            serde_json::from_str::<RoutedMessage>(&plain_json).unwrap().skill_id,
            None
        );
    }

    #[test]
    fn message_can_carry_memory_reference() {
        let m = RoutedMessage::new("ws1", "a1", "a2", MessageKind::Context, "x", 123)
            .with_memory("mem-1");
        assert_eq!(m.memory_id.as_deref(), Some("mem-1"));

        let json = serde_json::to_string(&m).unwrap();
        let back: RoutedMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.memory_id.as_deref(), Some("mem-1"));

        let plain = RoutedMessage::new("ws1", "a1", "a2", MessageKind::Context, "x", 123);
        let plain_json = serde_json::to_string(&plain).unwrap();
        assert!(!plain_json.contains("memory_id"));
        assert_eq!(
            serde_json::from_str::<RoutedMessage>(&plain_json).unwrap().memory_id,
            None
        );
    }

    #[test]
    fn route_to_peer_blocks_self_message() {
        let a = cli_agent_named("a1", "OpenCode");
        let b = cli_agent_named("a2", "Cline");
        let agents = vec![a, b];
        let edges = vec![sample_edge("a1", "a2")];
        let transport = Arc::new(MockTransport::new(&["a2"]));
        let router = ContextRouter::new(transport);

        // Ciclo básico: origem == destino (por id).
        let report = router.route_to_peer(None, "ws1", &agents, &edges, "a1", "a1", "x");
        assert!(!report.delivered);
        assert!(report.error.as_deref().unwrap().contains("si mesmo"));
    }

    #[test]
    fn route_a_to_b_flow_delivers_exact_payload() {
        let a = cli_agent("a1");
        let b = cli_agent("a2");
        let agents = vec![a, b];
        let edges = vec![sample_edge("a1", "a2")];
        let transport = Arc::new(MockTransport::new(&["a2"]));
        let router = ContextRouter::new(transport.clone());

        // Fluxo A -> B: origem a1, destino conectado a2, payload preservado.
        let reports = router.route(None, "ws1", &agents, &edges, "a1", "olá A -> B");

        assert_eq!(reports.len(), 1);
        assert!(reports[0].delivered);
        assert_eq!(reports[0].target, "a2");
        assert_eq!(transport.delivered_targets(), vec!["a2".to_string()]);
        assert_eq!(transport.delivered_payloads(), vec!["olá A -> B".to_string()]);
    }

    #[test]
    fn route_passes_workspace_id_to_transport() {
        let a = cli_agent("a1");
        let b = cli_agent("a2");
        let agents = vec![a, b];
        let edges = vec![sample_edge("a1", "a2")];
        let transport = Arc::new(MockTransport::new(&["a2"]));
        let router = ContextRouter::new(transport.clone());

        let _ = router.route(None, "ws-42", &agents, &edges, "a1", "oi");

        assert_eq!(transport.seen_workspaces(), vec!["ws-42".to_string()]);
    }

    #[test]
    fn route_to_peer_passes_workspace_id_to_transport() {
        let a = cli_agent_named("a1", "OpenCode");
        let b = cli_agent_named("a2", "Cline");
        let agents = vec![a, b];
        let edges = vec![sample_edge("a1", "a2")];
        let transport = Arc::new(MockTransport::new(&["a2"]));
        let router = ContextRouter::new(transport.clone());

        let report = router.route_to_peer(None, "ws-42", &agents, &edges, "a1", "Cline", "oi");

        assert!(report.delivered);
        assert_eq!(transport.seen_workspaces(), vec!["ws-42".to_string()]);
    }

    #[test]
    fn round_trip_ab_and_ba_delivers_both_ways() {
        let opencode = cli_agent_named("a1", "OpenCode");
        let cline = cli_agent_named("a2", "Cline");
        let agents = vec![opencode, cline];
        let edges = vec![sample_edge("a1", "a2")];
        let transport = Arc::new(MockTransport::new(&["a1", "a2"]));
        let router = ContextRouter::new(transport.clone());

        // A -> B
        let r1 = router.route_to_peer(None, "ws1", &agents, &edges, "a1", "Cline", "olá");
        assert!(r1.delivered);
        assert_eq!(r1.target, "a2");

        // B -> A (resposta pelo mesmo canal/topologia)
        let r2 = router.route_to_peer(None, "ws1", &agents, &edges, "a2", "OpenCode", "oi de volta");
        assert!(r2.delivered);
        assert_eq!(r2.target, "a1");

        // A sequência de entregas prova o ciclo A -> B -> A no mesmo workspace.
        assert_eq!(
            transport.delivered_pairs(),
            vec![
                ("a2".to_string(), "olá".to_string()),
                ("a1".to_string(), "oi de volta".to_string()),
            ]
        );
        assert_eq!(transport.seen_workspaces(), vec!["ws1".to_string(), "ws1".to_string()]);
    }

    #[test]
    fn route_to_peer_rejects_when_transport_workspace_mismatches() {
        let opencode = cli_agent_named("a1", "OpenCode");
        let cline = cli_agent_named("a2", "Cline");
        let agents = vec![opencode, cline];
        let edges = vec![sample_edge("a1", "a2")];

        // O transporte simula um processo que só existe no workspace "ws-A".
        let transport = Arc::new(MockTransport::new(&["a2"]).with_workspace("ws-A"));
        let router = ContextRouter::new(transport);

        // Mesmo workspace: entrega.
        let ok = router.route_to_peer(None, "ws-A", &agents, &edges, "a1", "Cline", "x");
        assert!(ok.delivered);

        // Workspace diferente: NÃO entrega (isolamento entre workspaces).
        let leak = router.route_to_peer(None, "ws-B", &agents, &edges, "a1", "Cline", "x");
        assert!(!leak.delivered);
        assert!(leak.error.as_deref().unwrap().contains("workspace"));
    }

    fn cli_agent_named(id: &str, name: &str) -> Agent {
        let mut a = cli_agent(id);
        a.name = name.to_string();
        a
    }

    #[test]
    fn parse_protocol_line_send_and_peers() {
        match parse_protocol_line("[[MESTRO:send Cline]] olá mundo\n").unwrap() {
            ProtocolDirective::Send { target, payload } => {
                assert_eq!(target, "Cline");
                assert_eq!(payload, "olá mundo");
            }
            _ => panic!("esperado Send"),
        }
        assert_eq!(
            parse_protocol_line("[[MESTRO:peers]]\n"),
            Some(ProtocolDirective::Peers)
        );
        assert!(parse_protocol_line("[[MESTRO:send ]] x").is_none());
        assert!(parse_protocol_line("normal").is_none());
        assert!(parse_protocol_line("[[MESTRO:send Cline]] ").is_none());
    }

    #[test]
    fn scanner_detects_send_directive_split_across_chunks() {
        let mut sc = ProtocolScanner::new();
        let r1 = sc.feed("[[MESTRO:s");
        assert!(r1.directives.is_empty());
        assert!(r1.forward.is_empty());
        let r2 = sc.feed("end Cline]] hello world\n");
        assert_eq!(r2.directives.len(), 1);
        assert_eq!(
            r2.directives[0],
            ProtocolDirective::Send {
                target: "Cline".to_string(),
                payload: "hello world".to_string()
            }
        );
        assert!(r2.forward.is_empty());
    }

    #[test]
    fn scanner_forwards_normal_output_immediately() {
        let mut sc = ProtocolScanner::new();
        let r = sc.feed("hello");
        assert_eq!(r.forward, "hello");
        assert!(r.directives.is_empty());
        let r2 = sc.feed(" world\nnext");
        assert_eq!(r2.forward, " world\nnext");
        assert!(r2.directives.is_empty());
    }

    #[test]
    fn scanner_routes_directive_and_forwards_rest() {
        let mut sc = ProtocolScanner::new();
        let r = sc.feed("normal line\n[[MESTRO:send Cline]] hi\nnormal2");
        assert_eq!(r.forward, "normal line\nnormal2");
        assert_eq!(r.directives.len(), 1);
        assert_eq!(
            r.directives[0],
            ProtocolDirective::Send {
                target: "Cline".to_string(),
                payload: "hi".to_string()
            }
        );
    }

    #[test]
    fn scanner_detects_peers_query() {
        let mut sc = ProtocolScanner::new();
        let r = sc.feed("[[MESTRO:peers]]\n");
        assert_eq!(r.forward, "");
        assert_eq!(r.directives, vec![ProtocolDirective::Peers]);
    }

    #[test]
    fn resolve_peer_id_by_name_or_id() {
        let a = cli_agent_named("a1", "OpenCode");
        let b = cli_agent_named("a2", "Cline");
        let agents = vec![a, b];
        let edges = vec![sample_edge("a1", "a2")];
        assert_eq!(resolve_peer_id(&agents, &edges, "a1", "a2"), Some("a2".to_string()));
        assert_eq!(resolve_peer_id(&agents, &edges, "a1", "Cline"), Some("a2".to_string()));
        assert_eq!(resolve_peer_id(&agents, &edges, "a1", "cline"), Some("a2".to_string()));
    }

    #[test]
    fn connected_peer_ids_are_bidirectional() {
        let edges = vec![sample_edge("a1", "a2")];
        assert_eq!(connected_peer_ids(&edges, "a1"), vec!["a2".to_string()]);
        assert_eq!(connected_peer_ids(&edges, "a2"), vec!["a1".to_string()]);
    }

    #[test]
    fn resolve_peer_id_is_bidirectional() {
        let a = cli_agent_named("a1", "OpenCode");
        let b = cli_agent_named("a2", "Cline");
        let agents = vec![a, b];
        let edges = vec![sample_edge("a1", "a2")];
        // Relação conectada é não-direcional: os dois se enxergam.
        assert_eq!(resolve_peer_id(&agents, &edges, "a1", "Cline"), Some("a2".to_string()));
        assert_eq!(resolve_peer_id(&agents, &edges, "a2", "OpenCode"), Some("a1".to_string()));
    }

    #[test]
    fn resolve_peer_id_rejects_non_connected() {
        let a = cli_agent_named("a1", "OpenCode");
        let b = cli_agent_named("a2", "Cline");
        let agents = vec![a, b];
        let edges = vec![sample_edge("a1", "a2")];
        assert_eq!(resolve_peer_id(&agents, &edges, "a1", "ghost"), None);
    }

    #[test]
    fn build_peers_banner_lists_connected_peers() {
        let a = cli_agent_named("a1", "OpenCode");
        let b = cli_agent_named("a2", "Cline");
        let agents = vec![a, b];
        let edges = vec![sample_edge("a1", "a2")];
        let banner = build_peers_banner(&agents, &edges, "a1");
        assert!(banner.contains("Cline"));
        assert!(banner.contains("id=a2"));
        assert!(banner.contains("[[MESTRO:send"));
    }

    #[test]
    fn build_peers_banner_sees_reverse_direction() {
        let a = cli_agent_named("a1", "OpenCode");
        let b = cli_agent_named("a2", "Cline");
        let agents = vec![a, b];
        let edges = vec![sample_edge("a1", "a2")];
        // Mesmo sendo o target da edge, a2 vê a1 (e vice-versa é coberto acima).
        let banner = build_peers_banner(&agents, &edges, "a2");
        assert!(banner.contains("OpenCode"));
        assert!(banner.contains("id=a1"));
    }

    #[test]
    fn build_peers_banner_identity_without_edges() {
        let a = cli_agent_named("a1", "OpenCode");
        let b = cli_agent_named("a2", "Cline");
        let agents = vec![a, b];
        let edges: Vec<Edge> = vec![];
        // Sem edges, ainda anuncia a identidade do MAESTRO 2.0 (nunca vazio),
        // para que o agente saiba onde está mesmo sem peers conectados.
        let banner = build_peers_banner(&agents, &edges, "a1");
        assert!(!banner.is_empty());
        assert!(banner.contains("MAESTRO 2.0"));
        assert!(banner.contains("Connected peers: none"));
    }

    #[test]
    fn banner_identifies_maestro2_canvas_and_forbids_external_discovery() {
        let a = cli_agent_named("a1", "OpenCode");
        let b = cli_agent_named("a2", "Cline");
        let agents = vec![a, b];
        let edges = vec![sample_edge("a1", "a2")];
        let banner = build_peers_banner(&agents, &edges, "a1");

        // Identidade explícita do MAESTRO 2.0 e fonte de verdade = canvas.
        assert!(banner.contains("MAESTRO 2.0"));
        assert!(banner.contains("single source of truth"));

        // Proíbe explicitamente ferramentas externas de descoberta.
        assert!(banner.contains("maestri list"));
        assert!(banner.contains("maestri ask"));
        assert!(banner.contains("OmniRoute"));

        // Peers e protocolo interno presentes.
        assert!(banner.contains("Cline"));
        assert!(banner.contains("[[MESTRO:peers]]"));
        assert!(banner.contains("[[MESTRO:send"));
    }

    #[test]
    fn banner_echo_does_not_trigger_directives() {
        let a = cli_agent_named("a1", "OpenCode");
        let b = cli_agent_named("a2", "Cline");
        let agents = vec![a, b];
        let edges = vec![sample_edge("a1", "a2")];
        let banner = build_peers_banner(&agents, &edges, "a1");

        // Se o agente ecoar o banner de volta, o scanner NÃO deve interpretar
        // nenhuma linha como diretiva de send/list (sem loop, sem eco em cascata).
        let mut sc = ProtocolScanner::new();
        let res = sc.feed(&banner);
        assert!(res.directives.is_empty());
        assert!(!res.forward.is_empty());
    }

    #[test]
    fn parse_protocol_line_ignores_ansi_and_leading_ws() {
        match parse_protocol_line("\u{1b}[1;32m[[MESTRO:send Cline]] hi\u{1b}[0m\n").unwrap() {
            ProtocolDirective::Send { target, payload } => {
                assert_eq!(target, "Cline");
                assert_eq!(payload, "hi");
            }
            _ => panic!("esperado Send"),
        }
        assert_eq!(
            parse_protocol_line("   [[MESTRO:peers]]  "),
            Some(ProtocolDirective::Peers)
        );
    }

    #[test]
    fn strip_ansi_removes_escape_sequences() {
        assert_eq!(strip_ansi("a\u{1b}[31mb\u{1b}[0mc"), "abc");
        assert_eq!(strip_ansi("plain"), "plain");
    }

    #[test]
    fn parse_protocol_line_requires_line_start() {
        // Diretiva em meio a texto NÃO dispara (evita eco do banner).
        assert!(parse_protocol_line("[MESTRO] use [[MESTRO:send X]] y\n").is_none());
        assert!(parse_protocol_line("prefix [[MESTRO:peers]]\n").is_none());
    }
}