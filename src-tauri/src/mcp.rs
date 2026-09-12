//! MCP Manager (Fase 10) — camada de configuração/gerenciamento de servidores
//! MCP, integrada às Skills e ao Agent Protocol, sem sistema paralelo.
//!
//! Escopo desta fase:
//! - cadastro/gestão de servidores MCP (id, name, transport, config, enabled,
//!   description/metadata);
//! - associação MCP <-> Skill e MCP <-> Agent;
//! - listagem/consulta por workspace (isolamento);
//! - validação básica de configuração;
//! - persistência local (via WorkspaceState).
//!
//! Fora de escopo: executar ferramentas externas, protocolo MCP completo,
//! Marketplace, Cloud, Supabase, Memory, Web/App Agents, execução remota.

use crate::models::{McpServer, McpServerConfig, McpTransport};
use crate::state::AppState;
use serde::Deserialize;
use std::collections::HashMap;
use tauri::State;

/// Payload de criação/registro de um servidor MCP.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateMcpServerInput {
    pub name: String,
    pub transport: McpTransport,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub url: Option<String>,
    /// Default: true (servidor habilitado). Use false para criar desabilitado.
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub description: Option<String>,
    /// Opcional: agente ao qual associar já na criação.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Opcional: skill à qual associar já na criação.
    #[serde(default)]
    pub skill_id: Option<String>,
}

/// Lista os servidores MCP do workspace atual.
#[tauri::command]
pub fn list_mcp_servers(state: State<'_, AppState>) -> Vec<McpServer> {
    let guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    guard.mcp_servers.clone()
}

/// Consulta os servidores MCP associados a um agente do workspace atual.
#[tauri::command]
pub fn mcps_of_agent(state: State<'_, AppState>, agent_id: String) -> Vec<McpServer> {
    let guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    if !guard.agents.iter().any(|a| a.id == agent_id) {
        return Vec::new();
    }
    guard.mcps_of_agent(&agent_id)
}

/// Consulta os servidores MCP associados a uma skill do workspace atual.
#[tauri::command]
pub fn mcps_of_skill(state: State<'_, AppState>, skill_id: String) -> Vec<McpServer> {
    let guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    if guard.get_skill(&skill_id).is_none() {
        return Vec::new();
    }
    guard.mcps_of_skill(&skill_id)
}

/// Cadastra um servidor MCP (valida config, opcionalmente associa).
#[tauri::command]
pub fn create_mcp_server(
    state: State<'_, AppState>,
    input: CreateMcpServerInput,
) -> Result<McpServer, String> {
    let server = McpServer {
        id: crate::workflow::gen_id("mcp"),
        name: input.name.trim().to_string(),
        transport: input.transport,
        config: McpServerConfig {
            command: input.command.clone(),
            args: input.args.clone(),
            url: input.url.clone(),
            env: HashMap::new(),
        },
        enabled: input.enabled.unwrap_or(true),
        description: input.description.clone(),
        metadata: HashMap::new(),
    };
    server.validate()?;

    let mut guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    if let Some(agent_id) = &input.agent_id {
        if !guard.agents.iter().any(|a| &a.id == agent_id) {
            return Err("agente não encontrado no workspace".to_string());
        }
    }
    if let Some(skill_id) = &input.skill_id {
        if guard.get_skill(skill_id).is_none() {
            return Err("skill não encontrada no workspace".to_string());
        }
    }

    guard.mcp_servers.push(server.clone());
    if let Some(agent_id) = &input.agent_id {
        guard.associate_mcp_to_agent(agent_id, &server.id);
    }
    if let Some(skill_id) = &input.skill_id {
        guard.associate_mcp_to_skill(skill_id, &server.id);
    }
    drop(guard);

    state.persist()?;
    Ok(server)
}

/// Associa um servidor MCP a um agente do workspace atual.
#[tauri::command]
pub fn associate_mcp_to_agent(
    state: State<'_, AppState>,
    agent_id: String,
    server_id: String,
) -> Result<(), String> {
    let mut guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    if !guard.agents.iter().any(|a| a.id == agent_id) {
        return Err("agente não encontrado no workspace".to_string());
    }
    if guard.get_mcp(&server_id).is_none() {
        return Err("MCP server não encontrado no workspace".to_string());
    }
    guard.associate_mcp_to_agent(&agent_id, &server_id);
    drop(guard);
    state.persist()
}

/// Associa um servidor MCP a uma skill do workspace atual.
#[tauri::command]
pub fn associate_mcp_to_skill(
    state: State<'_, AppState>,
    skill_id: String,
    server_id: String,
) -> Result<(), String> {
    let mut guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    if guard.get_skill(&skill_id).is_none() {
        return Err("skill não encontrada no workspace".to_string());
    }
    if guard.get_mcp(&server_id).is_none() {
        return Err("MCP server não encontrado no workspace".to_string());
    }
    guard.associate_mcp_to_skill(&skill_id, &server_id);
    drop(guard);
    state.persist()
}

/// Remove um servidor MCP do workspace (e de todas as associações).
#[tauri::command]
pub fn delete_mcp_server(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    let before = guard.mcp_servers.len();
    guard.mcp_servers.retain(|m| m.id != id);
    if guard.mcp_servers.len() == before {
        return Err("MCP server não encontrado".to_string());
    }
    for ids in guard.agent_mcp.values_mut() {
        ids.retain(|m| m != &id);
    }
    for ids in guard.skill_mcp.values_mut() {
        ids.retain(|m| m != &id);
    }
    drop(guard);
    state.persist()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Agent, AgentKind, Role, Runtime, Status, WorkspaceState};
    use crate::persistence::Persistence;

    fn agent(id: &str) -> Agent {
        Agent {
            id: id.to_string(),
            name: id.to_string(),
            role: Role::Builder,
            runtime: Runtime::Custom,
            kind: AgentKind::Cli,
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

    fn stdio_server(id: &str, name: &str) -> McpServer {
        McpServer {
            id: id.to_string(),
            name: name.to_string(),
            transport: McpTransport::Stdio,
            config: McpServerConfig {
                command: Some("node".to_string()),
                args: Some(vec!["server.js".to_string()]),
                url: None,
                env: HashMap::new(),
            },
            enabled: true,
            description: None,
            metadata: HashMap::new(),
        }
    }

    fn temp_dir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "maestri-mcp-{}-{:?}",
            SEQ.fetch_add(1, Ordering::SeqCst),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn mcp_server_serializes_roundtrip() {
        let s = stdio_server("m1", "Filesystem");
        let json = serde_json::to_string(&s).unwrap();
        let back: McpServer = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn mcp_server_validate_config() {
        // stdio sem command => inválido.
        let bad = McpServer {
            id: "x".into(),
            name: "x".into(),
            transport: McpTransport::Stdio,
            config: McpServerConfig::default(),
            enabled: true,
            description: None,
            metadata: HashMap::new(),
        };
        assert!(bad.validate().is_err());

        // http sem url => inválido.
        let bad_http = McpServer {
            id: "y".into(),
            name: "y".into(),
            transport: McpTransport::Http,
            config: McpServerConfig::default(),
            enabled: true,
            description: None,
            metadata: HashMap::new(),
        };
        assert!(bad_http.validate().is_err());

        // válido (stdio com command).
        assert!(stdio_server("m1", "F").validate().is_ok());
    }

    #[test]
    fn mcp_association_to_agent_and_skill() {
        let mut ws = WorkspaceState::new("ws".into());
        ws.agents.push(agent("a1"));
        ws.skills.push(crate::models::Skill {
            id: "sk1".into(),
            name: "Code".into(),
            description: "d".into(),
            version: None,
        });
        ws.mcp_servers.push(stdio_server("m1", "Filesystem"));

        ws.associate_mcp_to_agent("a1", "m1");
        ws.associate_mcp_to_skill("sk1", "m1");

        assert_eq!(ws.mcps_of_agent("a1").len(), 1);
        assert_eq!(ws.mcps_of_skill("sk1").len(), 1);
        assert_eq!(ws.mcps_of_agent("a1")[0].id, "m1");
        assert_eq!(ws.mcps_of_skill("sk1")[0].id, "m1");

        // idempotente.
        assert!(!ws.associate_mcp_to_agent("a1", "m1"));
        assert_eq!(ws.mcps_of_agent("a1").len(), 1);
    }

    #[test]
    fn mcp_isolated_between_workspaces() {
        let mut ws_a = WorkspaceState::new("A".into());
        ws_a.agents.push(agent("a1"));
        ws_a.mcp_servers.push(stdio_server("m1", "OnlyA"));
        ws_a.associate_mcp_to_agent("a1", "m1");

        let mut ws_b = WorkspaceState::new("B".into());
        ws_b.agents.push(agent("a1")); // mesmo id, outro workspace

        assert_eq!(ws_a.mcps_of_agent("a1").len(), 1);
        assert!(ws_b.mcps_of_agent("a1").is_empty());
        assert!(ws_b.get_mcp("m1").is_none());
    }

    #[test]
    fn disabled_server_is_preserved() {
        let mut enabled = stdio_server("m1", "F");
        enabled.enabled = false;

        let json = serde_json::to_string(&enabled).unwrap();
        let back: McpServer = serde_json::from_str(&json).unwrap();
        assert!(!back.enabled);

        // Default (campo omitido) => true.
        let defaulted: McpServer = serde_json::from_str(
            r#"{"id":"m2","name":"X","transport":"stdio","config":{"command":"n"}}"#,
        )
        .unwrap();
        assert!(defaulted.enabled);
    }

    #[test]
    fn mcp_persistence_reload() {
        let dir = temp_dir();
        let p = Persistence::new(dir.clone());

        let mut ws = WorkspaceState::new("MCP WS".to_string());
        ws.agents.push(agent("a1"));
        ws.mcp_servers.push(stdio_server("m1", "Filesystem"));
        ws.associate_mcp_to_agent("a1", "m1");
        p.save_workspace(&ws).unwrap();

        // Reload (simula reinício).
        let loaded = p.load_workspace(&ws.metadata.id).unwrap().unwrap();
        assert_eq!(loaded.mcp_servers.len(), 1);
        assert_eq!(loaded.mcp_servers[0].id, "m1");
        assert_eq!(loaded.mcps_of_agent("a1").len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}