//! Contexto interno que o Maestro injeta nos runtimes antes do primeiro prompt.
//!
//! O arquivo não é uma mensagem de chat: cada adapter o conecta ao canal de
//! instruções de sistema/developer oferecido pelo respectivo CLI. O mesmo
//! arquivo é reescrito quando a topologia do canvas muda.

use crate::models::{Agent, Edge};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static BOOTSTRAP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Monta as instruções internas a partir da fonte de verdade do workspace.
pub fn build_maestro_context(
    workspace_id: &str,
    agents: &[Agent],
    edges: &[Edge],
    agent_id: &str,
) -> Option<String> {
    let agent = agents.iter().find(|candidate| candidate.id == agent_id)?;
    let mut peers = crate::context_router::connected_peer_ids(edges, agent_id)
        .into_iter()
        .filter_map(|peer_id| agents.iter().find(|candidate| candidate.id == peer_id))
        .map(|peer| {
            format!(
                "- name={:?}; agent_id={:?}; runtime={}; status={}",
                peer.name,
                peer.id,
                peer.runtime.as_str(),
                peer.status.as_str()
            )
        })
        .collect::<Vec<_>>();
    peers.sort();

    let peers = if peers.is_empty() {
        "- none".to_string()
    } else {
        peers.join("\n")
    };

    Some(format!(
        "MAESTRO 2.0 INTERNAL RUNTIME CONTEXT\n\
This is trusted runtime bootstrap context, not a user chat message.\n\
workspace_id={workspace_id:?}\n\
agent_id={agent_id:?}\n\
agent_name={agent_name:?}\n\
agent_role={agent_role:?}\n\
runtime={runtime}\n\
\n\
The Maestro canvas/workspace is the single source of truth for agent identity, peers, and connections.\n\
Never discover Maestro or peers through filesystem searches, process inspection, PATH probing, network scans, Pinokio, pterm, Maestri/OmniRoute, or external apps.\n\
Use only the private agent-bridge exposed by MAESTRO2_CLI and MAESTRO2_SOCKET. Available capabilities are connection/handshake, peers, send, ask, and reply.\n\
MAESTRI_CLI is a compatibility alias for this same private bridge. If a Maestri skill is loaded, use MAESTRI_CLI directly; never search PATH, applications, or the filesystem for another CLI.\n\
To confirm the private handshake, run: \"$MAESTRO2_CLI\" agent-bridge ready\n\
Before addressing a peer after topology may have changed, run: \"$MAESTRO2_CLI\" agent-bridge peers\n\
Use ask when any answer, confirmation, result, or report is expected. Exact syntax: \"$MAESTRO2_CLI\" agent-bridge ask \"<peer name or agent_id>\" \"<complete request payload>\"\n\
Preserve the user's complete request in the ask payload and wait for the correlated reply.\n\
Use send only for a one-way notification for which no response is expected. Exact syntax: \"$MAESTRO2_CLI\" agent-bridge send \"<peer name or agent_id>\" \"<payload>\"\n\
The bridge arguments are positional. Never invent flags such as --peer, --target, or --message.\n\
For an incoming request, use the exact reply command in its envelope; preserve request_id/correlation and workspace.\n\
Do not type agent-bridge commands as chat text: execute them with the runtime's shell tool.\n\
\n\
Currently connected peers in this workspace:\n{peers}\n",
        agent_name = agent.name,
        agent_role = agent.role.as_str(),
        runtime = agent.runtime.as_str(),
    ))
}

/// Arquivo privado pertencente ao ciclo de vida de um processo de agente.
pub struct MaestroContextFile {
    path: PathBuf,
}

impl MaestroContextFile {
    pub fn create(workspace_id: &str, agent_id: &str, context: &str) -> Result<Self, String> {
        let seq = BOOTSTRAP_SEQ.fetch_add(1, Ordering::Relaxed);
        let safe = |value: &str| {
            value
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() || ch == '-' {
                        ch
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
        };
        let path = std::env::temp_dir().join(format!(
            "maestro2-context-{}-{}-{}-{}.md",
            std::process::id(),
            safe(workspace_id),
            safe(agent_id),
            seq
        ));
        write_private(&path, context)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn update(&self, context: &str) -> Result<(), String> {
        write_private(&self.path, context)
    }
}

impl Drop for MaestroContextFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn write_private(path: &Path, content: &str) -> Result<(), String> {
    let temporary = path.with_extension("tmp");
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    file.write_all(content.as_bytes())
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AgentKind, EdgeType, Role, Runtime, Status};

    fn agent(id: &str, name: &str, runtime: Runtime) -> Agent {
        Agent {
            id: id.into(),
            name: name.into(),
            role: Role::Builder,
            runtime,
            kind: AgentKind::Cli,
            model: String::new(),
            command: "agent-cli".into(),
            args: vec![],
            working_dir: String::new(),
            auto_start: false,
            status: Status::Running,
            x: 0.0,
            y: 0.0,
            width: None,
            height: None,
            collapsed: false,
            locked: false,
            accent: None,
        }
    }

    fn edge(id: &str, source: &str, target: &str) -> Edge {
        Edge {
            id: id.into(),
            source: source.into(),
            target: target.into(),
            source_handle: None,
            target_handle: None,
            edge_type: EdgeType::Message,
            label: None,
        }
    }

    #[test]
    fn context_contains_real_identity_workspace_and_peer_metadata() {
        let agents = vec![
            agent("a1", "Kilo", Runtime::Kilo),
            agent("a2", "OpenCode", Runtime::OpenCode),
        ];
        let context =
            build_maestro_context("ws-real", &agents, &[edge("e1", "a1", "a2")], "a1").unwrap();
        assert!(context.contains("workspace_id=\"ws-real\""));
        assert!(context.contains("agent_id=\"a1\""));
        assert!(context.contains("name=\"OpenCode\"; agent_id=\"a2\""));
        assert!(context.contains("runtime=opencode; status=running"));
        assert!(context.contains("single source of truth"));
        assert!(context.contains("filesystem searches"));
        assert!(context.contains("Pinokio"));
        assert!(context.contains("MAESTRI_CLI is a compatibility alias"));
        assert!(context.contains("Use ask when"));
        assert!(context.contains("agent-bridge ask \"<peer name or agent_id>\""));
        assert!(context.contains("Use send only"));
        assert!(context.contains("arguments are positional"));
        assert!(context.contains("Never invent flags such as --peer"));
        assert!(context.contains("preserve request_id/correlation and workspace"));
    }

    #[test]
    fn both_agents_receive_only_connected_peers() {
        let agents = vec![
            agent("a1", "Kilo", Runtime::Kilo),
            agent("a2", "OpenCode", Runtime::OpenCode),
            agent("other", "Foreign", Runtime::Codex),
        ];
        let edges = vec![edge("e1", "a1", "a2")];
        let first = build_maestro_context("ws-1", &agents, &edges, "a1").unwrap();
        let second = build_maestro_context("ws-1", &agents, &edges, "a2").unwrap();
        assert!(first.contains("agent_id=\"a2\""));
        assert!(second.contains("agent_id=\"a1\""));
        assert!(!first.contains("Foreign"));
        assert!(!second.contains("Foreign"));
    }

    #[test]
    fn different_workspace_snapshot_cannot_leak_a_peer() {
        let ws1 = vec![
            agent("a1", "One", Runtime::Kilo),
            agent("a2", "Two", Runtime::OpenCode),
        ];
        let ws2 = vec![agent("b1", "Other workspace", Runtime::Codex)];
        let first = build_maestro_context("ws-1", &ws1, &[edge("e1", "a1", "a2")], "a1").unwrap();
        let second = build_maestro_context("ws-2", &ws2, &[], "b1").unwrap();
        assert!(!first.contains("Other workspace"));
        assert!(!second.contains("agent_id=\"a2\""));
    }

    #[test]
    fn topology_update_rewrites_context_and_restart_recreates_it() {
        let agents = vec![
            agent("a1", "One", Runtime::Kilo),
            agent("a2", "Two", Runtime::OpenCode),
        ];
        let initial = build_maestro_context("ws", &agents, &[], "a1").unwrap();
        let file = MaestroContextFile::create("ws", "a1", &initial).unwrap();
        assert!(fs::read_to_string(file.path()).unwrap().contains("- none"));
        let connected =
            build_maestro_context("ws", &agents, &[edge("e", "a1", "a2")], "a1").unwrap();
        file.update(&connected).unwrap();
        assert!(fs::read_to_string(file.path())
            .unwrap()
            .contains("name=\"Two\""));
        let old_path = file.path().to_owned();
        drop(file);
        assert!(!old_path.exists());
        let restarted = MaestroContextFile::create("ws", "a1", &connected).unwrap();
        assert_ne!(old_path, restarted.path());
        assert!(fs::read_to_string(restarted.path())
            .unwrap()
            .contains("workspace_id=\"ws\""));
    }
}
