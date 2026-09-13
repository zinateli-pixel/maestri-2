use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Papel (role) de um agente no workspace.
/// Aplicado no backend — não apenas no prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Orchestrator,
    Specialist,
    Auditor,
    Builder,
    Reviewer,
    Qa,
    Observer,
}

impl Role {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Orchestrator => "orchestrator",
            Role::Specialist => "specialist",
            Role::Auditor => "auditor",
            Role::Builder => "builder",
            Role::Reviewer => "reviewer",
            Role::Qa => "qa",
            Role::Observer => "observer",
        }
    }
}

/// Runtime é um conceito independente do modelo de dados.
/// Define QUAL motor executa o agente (Kilo, Claude Code, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    Kilo,
    ClaudeCode,
    #[serde(rename = "opencode")]
    OpenCode,
    Codex,
    Custom,
}

impl Runtime {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            Runtime::Kilo => "kilo",
            Runtime::ClaudeCode => "claude_code",
            Runtime::OpenCode => "opencode",
            Runtime::Codex => "codex",
            Runtime::Custom => "custom",
        }
    }
}

/// Tipo/meio de execução de um agente.
///
/// Desacopla a identidade do agente de um runtime específico:
/// - `Cli` = agente executado via terminal/PTY (Kilo, Claude Code, Shell...);
/// - `Web`/`App` = reservados para fases futuras (agentes via navegador ou
///   aplicativo). A modelo fica preparado sem implementar esses meios agora.
///
/// A separação conceitual é: `kind` define ONDE/COMO o agente executa
/// (meio), enquanto `runtime` define QUAL motor concreto (relevante hoje
/// apenas para `Cli`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    #[default]
    Cli,
    Web,
    App,
}

impl AgentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentKind::Cli => "cli",
            AgentKind::Web => "web",
            AgentKind::App => "app",
        }
    }
}

/// Status de um agente no ciclo de vida do workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Idle,
    Starting,
    Running,
    Waiting,
    Done,
    Failed,
    Stopped,
}

impl Status {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Idle => "idle",
            Status::Starting => "starting",
            Status::Running => "running",
            Status::Waiting => "waiting",
            Status::Done => "done",
            Status::Failed => "failed",
            Status::Stopped => "stopped",
        }
    }
}

/// Um agente no canvas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub role: Role,
    pub runtime: Runtime,
    /// Tipo/meio de execução (cli, web, app). Desacopla a identidade do
    /// agente de um runtime específico; defaults para `cli` (compatível
    /// com workspaces persistidos antigos).
    #[serde(default)]
    pub kind: AgentKind,
    /// Modelo de IA usado pelo runtime (ex.: "deepseek-v4-pro").
    /// Conceito separado do runtime: runtime = motor, model = modelo.
    pub model: String,
    /// Comando executado pelo runtime (ex.: "claude").
    pub command: String,
    /// Argumentos passados ao comando.
    pub args: Vec<String>,
    /// Diretório de trabalho do agente.
    pub working_dir: String,
    /// Se o agente deve iniciar automaticamente ao carregar o workspace.
    #[serde(default)]
    pub auto_start: bool,
    pub status: Status,
    /// Posição no canvas (coordenadas normalizadas 0..1).
    pub x: f64,
    pub y: f64,
    /// Largura do node em px (None = default do frontend).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    /// Altura do node em px (None = default do frontend).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    /// Node colapsado (terminal oculto, processo continua rodando).
    #[serde(default)]
    pub collapsed: bool,
    /// Node travado (não arrasta nem redimensiona).
    #[serde(default)]
    pub locked: bool,
    /// Cor de destaque do node (hex). None = cor da role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
}

/// Uma conexão (edge) entre dois agentes no canvas.
/// Representa apenas a topologia visual — não é um canal de mensagens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    /// Id do agente de origem.
    pub source: String,
    /// Id do agente de destino.
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_handle: Option<String>,
    /// Tipo visual da conexão.
    #[serde(default = "default_edge_type")]
    pub edge_type: EdgeType,
    /// Label opcional da conexão.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

fn default_edge_type() -> EdgeType {
    EdgeType::Message
}

/// Tipo visual da conexão.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    Message,
    Data,
    Control,
    Workflow,
}

/// Uma Skill/capacidade declarada no workspace, associável a agentes.
/// Agnóstica ao runtime: não referencia Kilo/OpenCode/Shell — apenas declara
/// uma capacidade (id, nome, descrição, versão opcional).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Transporte de um servidor MCP. Desacoplado do runtime do agente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    Stdio,
    Sse,
    Http,
}

/// Configuração de conexão de um servidor MCP (por transporte).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// stdio: comando a executar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// stdio: argumentos do comando.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    /// http/sse: url de conexão.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Ambiente adicional (stdio).
    #[serde(default)]
    pub env: HashMap<String, String>,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            command: None,
            args: None,
            url: None,
            env: HashMap::new(),
        }
    }
}

/// Um servidor MCP cadastrado no workspace (camada de configuração, não
/// executa ferramentas externas automaticamente).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpServer {
    pub id: String,
    pub name: String,
    pub transport: McpTransport,
    #[serde(default)]
    pub config: McpServerConfig,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Metadados extras livres (chave/valor).
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl McpServer {
    /// Validação básica de configuração conforme o transporte.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("nome do MCP server é obrigatório".to_string());
        }
        match self.transport {
            McpTransport::Stdio => {
                if self.config.command.as_deref().map_or(true, |c| c.trim().is_empty()) {
                    return Err("transporte stdio requer 'command'".to_string());
                }
            }
            McpTransport::Http | McpTransport::Sse => {
                if self.config.url.as_deref().map_or(true, |u| u.trim().is_empty()) {
                    return Err("transporte http/sse requer 'url'".to_string());
                }
            }
        }
        Ok(())
    }
}

/// Valida uma conexão A → B antes de criá-la:
/// - self-loop é inválido;
/// - origem e destino precisam existir;
/// - edge duplicada (mesmo par source/target) é inválida.
/// Reutilizada por `add_edge` e pelos testes.
pub fn validate_edge(
    agents: &[Agent],
    edges: &[Edge],
    source: &str,
    target: &str,
) -> Result<(), String> {
    if source == target {
        return Err("não é possível conectar um agente a ele mesmo".to_string());
    }

    let has_source = agents.iter().any(|a| a.id == source);
    let has_target = agents.iter().any(|a| a.id == target);
    if !has_source || !has_target {
        return Err("agente de origem ou destino não encontrado".to_string());
    }

    if edges.iter().any(|e| e.source == source && e.target == target) {
        return Err("conexão já existe".to_string());
    }

    Ok(())
}

/// Viewport do canvas (posição e zoom).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Viewport {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

impl Default for Viewport {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0, zoom: 1.0 }
    }
}

/// Configurações do workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSettings {
    /// Grid visível no canvas.
    #[serde(default = "default_true")]
    pub show_grid: bool,
    /// Snap to grid.
    #[serde(default = "default_true")]
    pub snap_to_grid: bool,
    /// Tamanho do grid.
    #[serde(default = "default_grid_size")]
    pub grid_size: f64,
    /// Minimap visível.
    #[serde(default = "default_true")]
    pub show_minimap: bool,
    /// Auto-save habilitado.
    #[serde(default = "default_true")]
    pub auto_save: bool,
    /// Intervalo de auto-save em segundos.
    #[serde(default = "default_autosave_interval")]
    pub auto_save_interval: u64,
}

fn default_true() -> bool { true }
fn default_grid_size() -> f64 { 16.0 }
fn default_autosave_interval() -> u64 { 30 }

/// Metadados do workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMetadata {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub last_opened_at: u64,
}

/// Estado completo de um workspace individual.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub metadata: WorkspaceMetadata,
    pub agents: Vec<Agent>,
    /// Conexões entre agentes (criadas manualmente pelo usuário).
    #[serde(default)]
    pub edges: Vec<Edge>,
    /// Viewport do canvas.
    #[serde(default)]
    pub viewport: Viewport,
    /// Configurações do workspace.
    #[serde(default)]
    pub settings: WorkspaceSettings,
    /// Skills declaradas neste workspace (catálogo de capacidades).
    #[serde(default)]
    pub skills: Vec<Skill>,
    /// Associação agente -> ids de skills (agent_id -> Vec<skill_id>).
    #[serde(default)]
    pub agent_skills: HashMap<String, Vec<String>>,
    /// Servidores MCP cadastrados neste workspace.
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
    /// Associação agente -> ids de MCP servers.
    #[serde(default)]
    pub agent_mcp: HashMap<String, Vec<String>>,
    /// Associação skill -> ids de MCP servers.
    #[serde(default)]
    pub skill_mcp: HashMap<String, Vec<String>>,
    pub version: String,
}

/// Lista de workspaces para o seletor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceListItem {
    pub id: String,
    pub name: String,
    pub agent_count: usize,
    pub edge_count: usize,
    pub updated_at: u64,
    pub last_opened_at: u64,
}

/// Payload para criar um novo workspace.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateWorkspaceInput {
    pub name: String,
}

/// Payload para renomear workspace.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RenameWorkspaceInput {
    pub id: String,
    pub name: String,
}

/// Payload para atualizar configurações do workspace.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UpdateWorkspaceSettingsInput {
    pub id: String,
    pub settings: WorkspaceSettings,
}

/// Payload para atualizar viewport.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UpdateViewportInput {
    pub id: String,
    pub viewport: Viewport,
}

impl WorkspaceState {
    pub fn new(name: String) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let id = format!("workspace-{}", now);
        Self {
            metadata: WorkspaceMetadata {
                id: id.clone(),
                name,
                created_at: now,
                updated_at: now,
                last_opened_at: now,
            },
            agents: Vec::new(),
            edges: Vec::new(),
            viewport: Viewport::default(),
            settings: WorkspaceSettings::default(),
            skills: Vec::new(),
            agent_skills: HashMap::new(),
            mcp_servers: Vec::new(),
            agent_mcp: HashMap::new(),
            skill_mcp: HashMap::new(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub fn touch(&mut self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.metadata.updated_at = now;
    }

    pub fn mark_opened(&mut self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.metadata.last_opened_at = now;
    }

    pub fn to_list_item(&self) -> WorkspaceListItem {
        WorkspaceListItem {
            id: self.metadata.id.clone(),
            name: self.metadata.name.clone(),
            agent_count: self.agents.len(),
            edge_count: self.edges.len(),
            updated_at: self.metadata.updated_at,
            last_opened_at: self.metadata.last_opened_at,
        }
    }

    /// Consulta as skills associadas a um agente (por id), resolvidas a partir
    /// do catálogo do workspace.
    pub fn skills_of(&self, agent_id: &str) -> Vec<Skill> {
        let ids = self.agent_skills.get(agent_id).cloned().unwrap_or_default();
        ids.into_iter()
            .filter_map(|id| self.skills.iter().find(|s| s.id == id).cloned())
            .collect()
    }

    /// Retorna uma skill pelo id (Some/None), dentro deste workspace.
    pub fn get_skill(&self, id: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.id == id)
    }

    /// Associa uma skill a um agente (idempotente — sem duplicar).
    pub fn associate_skill(&mut self, agent_id: &str, skill_id: &str) -> bool {
        let ids = self.agent_skills.entry(agent_id.to_string()).or_default();
        if ids.iter().any(|s| s == skill_id) {
            return false;
        }
        ids.push(skill_id.to_string());
        true
    }

    /// Consulta os servidores MCP associados a um agente (por id).
    pub fn mcps_of_agent(&self, agent_id: &str) -> Vec<McpServer> {
        let ids = self.agent_mcp.get(agent_id).cloned().unwrap_or_default();
        ids.into_iter()
            .filter_map(|id| self.mcp_servers.iter().find(|m| m.id == id).cloned())
            .collect()
    }

    /// Consulta os servidores MCP associados a uma skill (por id).
    pub fn mcps_of_skill(&self, skill_id: &str) -> Vec<McpServer> {
        let ids = self.skill_mcp.get(skill_id).cloned().unwrap_or_default();
        ids.into_iter()
            .filter_map(|id| self.mcp_servers.iter().find(|m| m.id == id).cloned())
            .collect()
    }

    /// Retorna um servidor MCP pelo id (Some/None), dentro deste workspace.
    pub fn get_mcp(&self, id: &str) -> Option<&McpServer> {
        self.mcp_servers.iter().find(|m| m.id == id)
    }

    /// Associa um MCP server a um agente (idempotente).
    pub fn associate_mcp_to_agent(&mut self, agent_id: &str, mcp_id: &str) -> bool {
        let ids = self.agent_mcp.entry(agent_id.to_string()).or_default();
        if ids.iter().any(|m| m == mcp_id) {
            return false;
        }
        ids.push(mcp_id.to_string());
        true
    }

    /// Associa um MCP server a uma skill (idempotente).
    pub fn associate_mcp_to_skill(&mut self, skill_id: &str, mcp_id: &str) -> bool {
        let ids = self.skill_mcp.entry(skill_id.to_string()).or_default();
        if ids.iter().any(|m| m == mcp_id) {
            return false;
        }
        ids.push(mcp_id.to_string());
        true
    }
}

impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            show_grid: true,
            snap_to_grid: true,
            grid_size: 16.0,
            show_minimap: true,
            auto_save: true,
            auto_save_interval: 30,
        }
    }
}

/// Um Projeto agrupa múltiplos workspaces (Fase 12).
///
/// É uma camada organizacional sobre os workspaces existentes: a associação
/// é mantida somente no Projeto (`workspace_ids`), sem alterar os arquivos de
/// workspace — totalmente retrocompatível.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub workspace_ids: Vec<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl Project {
    pub fn new(name: String, now: u64) -> Self {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let c = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self {
            id: format!("project-{now}-{c}"),
            name,
            workspace_ids: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn touch(&mut self, now: u64) {
        self.updated_at = now;
    }

    /// Associa um workspace ao projeto (idempotente).
    pub fn add_workspace(&mut self, workspace_id: &str) -> bool {
        if self.workspace_ids.iter().any(|w| w == workspace_id) {
            return false;
        }
        self.workspace_ids.push(workspace_id.to_string());
        true
    }

    /// Desassocia um workspace do projeto. Retorna false se não estava associado.
    pub fn remove_workspace(&mut self, workspace_id: &str) -> bool {
        let before = self.workspace_ids.len();
        self.workspace_ids.retain(|w| w != workspace_id);
        self.workspace_ids.len() != before
    }
}

/// Payload para criar um projeto.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateProjectInput {
    pub name: String,
}

/// Payload para renomear um projeto.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RenameProjectInput {
    pub id: String,
    pub name: String,
}

/// Payload para associar/desassociar um workspace a um projeto.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProjectWorkspaceInput {
    pub project_id: String,
    pub workspace_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_agent() -> Agent {
        Agent {
            id: "a1".to_string(),
            name: "Test".to_string(),
            role: Role::Orchestrator,
            runtime: Runtime::Kilo,
            kind: AgentKind::Cli,
            model: "kilo".to_string(),
            command: "kilo".to_string(),
            args: vec![],
            working_dir: "".to_string(),
            auto_start: true,
            status: Status::Idle,
            x: 0.5,
            y: 0.5,
            width: None,
            height: None,
            collapsed: false,
            locked: false,
            accent: None,
        }
    }

    #[test]
    fn enums_serialize_snake_case() {
        assert_eq!(serde_json::to_string(&Role::Orchestrator).unwrap(), "\"orchestrator\"");
        assert_eq!(serde_json::to_string(&Role::Qa).unwrap(), "\"qa\"");
        assert_eq!(serde_json::to_string(&Runtime::ClaudeCode).unwrap(), "\"claude_code\"");
        assert_eq!(serde_json::to_string(&Runtime::Kilo).unwrap(), "\"kilo\"");
        assert_eq!(serde_json::to_string(&Runtime::OpenCode).unwrap(), "\"opencode\"");
        assert_eq!(serde_json::to_string(&Runtime::Codex).unwrap(), "\"codex\"");
        assert_eq!(serde_json::to_string(&Runtime::Custom).unwrap(), "\"custom\"");
        assert_eq!(serde_json::to_string(&Status::Starting).unwrap(), "\"starting\"");
        assert_eq!(serde_json::to_string(&EdgeType::Workflow).unwrap(), "\"workflow\"");
    }

    #[test]
    fn agent_kind_serializes_and_defaults_to_cli() {
        assert_eq!(serde_json::to_string(&AgentKind::Cli).unwrap(), "\"cli\"");
        assert_eq!(serde_json::to_string(&AgentKind::Web).unwrap(), "\"web\"");
        assert_eq!(serde_json::to_string(&AgentKind::App).unwrap(), "\"app\"");
        assert_eq!(AgentKind::default(), AgentKind::Cli);
        assert_eq!(AgentKind::Cli.as_str(), "cli");
    }

    #[test]
    fn agent_roundtrip_preserves_fields() {
        let agent = sample_agent();
        let json = serde_json::to_string(&agent).unwrap();
        let back: Agent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "a1");
        assert_eq!(back.role, Role::Orchestrator);
        assert_eq!(back.runtime, Runtime::Kilo);
        assert!(back.auto_start);
        assert_eq!(back.status, Status::Idle);
    }

    #[test]
    fn agent_deserializes_without_optional_fields() {
        // JSON legado sem width/height/collapsed/locked/accent/auto_start.
        let json = r#"{
            "id": "a1", "name": "Old", "role": "builder", "runtime": "custom",
            "model": "", "command": "sh", "args": [], "working_dir": "",
            "status": "idle", "x": 0.0, "y": 0.0
        }"#;
        let agent: Agent = serde_json::from_str(json).unwrap();
        assert_eq!(agent.width, None);
        assert!(!agent.collapsed);
        assert!(!agent.locked);
        assert!(!agent.auto_start);
        // Campo novo (kind) ausente em JSON legado deve assumir Cli.
        assert_eq!(agent.kind, AgentKind::Cli);
    }

    #[test]
    fn workspace_state_roundtrip() {
        let mut ws = WorkspaceState::new("Test".to_string());
        ws.agents.push(sample_agent());
        ws.edges.push(Edge {
            id: "e1".to_string(),
            source: "a1".to_string(),
            target: "a2".to_string(),
            source_handle: None,
            target_handle: None,
            edge_type: EdgeType::Message,
            label: None,
        });

        let json = serde_json::to_string_pretty(&ws).unwrap();
        let back: WorkspaceState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.metadata.name, "Test");
        assert_eq!(back.agents.len(), 1);
        assert_eq!(back.edges.len(), 1);
        assert_eq!(back.edges[0].edge_type, EdgeType::Message);
        assert!(back.settings.show_grid);
    }

    #[test]
    fn workspace_list_item_counts() {
        let mut ws = WorkspaceState::new("Counts".to_string());
        ws.agents.push(sample_agent());
        ws.edges.push(Edge {
            id: "e1".to_string(),
            source: "a1".to_string(),
            target: "a2".to_string(),
            source_handle: None,
            target_handle: None,
            edge_type: EdgeType::Data,
            label: None,
        });
        let item = ws.to_list_item();
        assert_eq!(item.agent_count, 1);
        assert_eq!(item.edge_count, 1);
    }

    #[test]
    fn touch_updates_timestamp() {
        let mut ws = WorkspaceState::new("T".to_string());
        let before = ws.metadata.updated_at;
        std::thread::sleep(std::time::Duration::from_millis(2));
        ws.touch();
        assert!(ws.metadata.updated_at >= before);
    }

    #[test]
    fn project_roundtrip_and_membership() {
        let mut p = Project::new("Novo Projeto".to_string(), 1000);
        assert_eq!(p.workspace_ids.len(), 0);
        assert!(p.add_workspace("ws-1"));
        assert!(!p.add_workspace("ws-1")); // idempotente
        assert!(p.add_workspace("ws-2"));
        assert_eq!(p.workspace_ids.len(), 2);

        let json = serde_json::to_string_pretty(&p).unwrap();
        let mut back: Project = serde_json::from_str(&json).unwrap();
        assert!(back.id.starts_with("project-1000"));
        assert_eq!(back.name, "Novo Projeto");
        assert_eq!(back.workspace_ids, vec!["ws-1", "ws-2"]);

        assert!(back.remove_workspace("ws-1"));
        assert!(!back.remove_workspace("ws-1")); // já removido
        assert_eq!(back.workspace_ids, vec!["ws-2"]);
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

    #[test]
    fn validate_edge_rejects_self_loop() {
        let agents = vec![sample_agent()];
        let edges: Vec<Edge> = vec![];
        let err = validate_edge(&agents, &edges, "a1", "a1").unwrap_err();
        assert!(err.contains("ele mesmo"));
    }

    #[test]
    fn validate_edge_rejects_missing_agent() {
        let agents = vec![sample_agent()];
        let edges: Vec<Edge> = vec![];
        assert!(validate_edge(&agents, &edges, "a1", "ghost").is_err());
        assert!(validate_edge(&agents, &edges, "ghost", "a1").is_err());
    }

    #[test]
    fn validate_edge_rejects_duplicate_but_allows_reverse() {
        let mut a2 = sample_agent();
        a2.id = "a2".to_string();
        let agents = vec![sample_agent(), a2];
        let edges = vec![sample_edge("a1", "a2")];
        assert!(validate_edge(&agents, &edges, "a1", "a2").is_err());
        // Sentido inverso é uma relação distinta, deve ser permitida.
        assert!(validate_edge(&agents, &edges, "a2", "a1").is_ok());
    }
}