//! Canal IPC confiável entre os CLIs de agentes e o runtime do Maestro 2.
//!
//! O stdout de TUIs (Kilo/OpenCode/Claude) é uma tela ANSI redesenhada, não um
//! fluxo de linhas estável. Por isso ele não pode ser a fonte primária de
//! comandos agent→agent. Cada processo recebe um socket Unix privado e o
//! próprio binário do app atua como cliente (`agent-bridge ...`).

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

#[cfg(unix)]
use std::sync::mpsc::Sender;
#[cfg(unix)]
use std::thread::JoinHandle;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum BridgeCommand {
    Ready,
    Peers,
    Send { target: String, payload: String },
    Ask { target: String, payload: String },
    Reply { correlation_id: String, payload: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeResponse {
    pub ok: bool,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl BridgeResponse {
    pub fn ok(message: impl Into<String>) -> Self {
        Self { ok: true, message: message.into(), request_id: None }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self { ok: false, message: message.into(), request_id: None }
    }
}

#[cfg(unix)]
pub fn run_client(args: &[String]) -> Result<(), String> {
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;

    let socket = std::env::var("MAESTRO2_SOCKET")
        .map_err(|_| "MAESTRO2_SOCKET ausente: este comando só funciona dentro de um agente do Maestro 2".to_string())?;
    let command = parse_client_args(args)?;
    let mut stream = UnixStream::connect(&socket)
        .map_err(|e| format!("falha ao conectar ao Maestro 2: {e}"))?;
    serde_json::to_writer(&mut stream, &command).map_err(|e| e.to_string())?;
    stream.write_all(b"\n").map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;
    stream.shutdown(Shutdown::Write).map_err(|e| e.to_string())?;

    let mut response = String::new();
    stream.read_to_string(&mut response).map_err(|e| e.to_string())?;
    print!("{}", response);
    let parsed: BridgeResponse = serde_json::from_str(response.trim()).map_err(|e| e.to_string())?;
    if parsed.ok { Ok(()) } else { Err(parsed.message) }
}

#[cfg(unix)]
pub struct BridgeServer {
    socket_path: std::path::PathBuf,
    shutdown: Sender<()>,
    thread: Option<JoinHandle<()>>,
}

#[cfg(unix)]
impl BridgeServer {
    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    pub fn stop(&mut self) {
        let _ = self.shutdown.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

#[cfg(unix)]
impl Drop for BridgeServer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(unix)]
pub fn start_server(
    app: tauri::AppHandle,
    workspace_id: String,
    agent_id: String,
) -> Result<BridgeServer, String> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;
    use std::time::Duration;

    let mut hasher = DefaultHasher::new();
    workspace_id.hash(&mut hasher);
    agent_id.hash(&mut hasher);
    let socket_path = std::env::temp_dir().join(format!(
        "maestro2-{}-{:x}.sock",
        std::process::id(),
        hasher.finish()
    ));
    let _ = std::fs::remove_file(&socket_path);
    let listener = UnixListener::bind(&socket_path)
        .map_err(|e| format!("falha ao criar bridge do agente: {e}"))?;
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("falha ao proteger bridge do agente: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;

    let (shutdown_tx, shutdown_rx) = mpsc::channel();
    let path_for_thread = socket_path.clone();
    let thread = std::thread::spawn(move || {
        loop {
            if shutdown_rx.try_recv().is_ok() {
                break;
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let response = read_command(&mut stream)
                        .and_then(|command| handle_command(&app, &workspace_id, &agent_id, command))
                        .unwrap_or_else(BridgeResponse::error);
                    let _ = serde_json::to_writer(&mut stream, &response);
                    let _ = stream.write_all(b"\n");
                    let _ = stream.flush();
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
        let _ = std::fs::remove_file(path_for_thread);
    });

    Ok(BridgeServer { socket_path, shutdown: shutdown_tx, thread: Some(thread) })
}

#[cfg(unix)]
fn read_command(stream: &mut std::os::unix::net::UnixStream) -> Result<BridgeCommand, String> {
    let mut input = String::new();
    stream.read_to_string(&mut input).map_err(|e| e.to_string())?;
    serde_json::from_str(input.trim()).map_err(|e| format!("comando inválido: {e}"))
}

#[cfg(unix)]
fn handle_command(
    app: &tauri::AppHandle,
    workspace_id: &str,
    agent_id: &str,
    command: BridgeCommand,
) -> Result<BridgeResponse, String> {
    use tauri::Manager;

    let state = app
        .try_state::<crate::state::AppState>()
        .ok_or_else(|| "estado do Maestro 2 indisponível".to_string())?;
    if state.current_workspace_id().as_deref() != Some(workspace_id)
        || !state.processes.is_running_in(workspace_id, agent_id)
    {
        return Err("agente não está RUNNING no workspace que criou esta bridge".to_string());
    }

    match command {
        BridgeCommand::Ready => {
            state.mark_protocol_ready(workspace_id, agent_id);
            Ok(BridgeResponse::ok(format!(
                "MAESTRO 2.0 ready; peers: {}",
                state.protocol_peer_names(agent_id).join(", ")
            )))
        }
        BridgeCommand::Peers => Ok(BridgeResponse::ok(state.protocol_peer_names(agent_id).join(", "))),
        BridgeCommand::Send { target, payload } => {
            let report = state.route_context_to_peer(app, agent_id, &target, &payload);
            if report.delivered {
                Ok(BridgeResponse::ok(format!("delivered to {}", report.target)))
            } else {
                Err(report.error.unwrap_or_else(|| "entrega falhou".to_string()))
            }
        }
        BridgeCommand::Ask { target, payload } => {
            let report = state.ask_peer(app, agent_id, &target, &payload);
            if report.delivered {
                Ok(BridgeResponse {
                    ok: true,
                    message: format!("request delivered to {}", report.target),
                    request_id: Some(report.request_id),
                })
            } else {
                Err(report.error.unwrap_or_else(|| "pedido falhou".to_string()))
            }
        }
        BridgeCommand::Reply { correlation_id, payload } => {
            let report = state.reply_to_request(app, agent_id, &correlation_id, &payload);
            if report.delivered {
                Ok(BridgeResponse::ok(format!("reply delivered to {}", report.target)))
            } else {
                Err(report.error.unwrap_or_else(|| "resposta falhou".to_string()))
            }
        }
    }
}

#[cfg(not(unix))]
pub fn run_client(_args: &[String]) -> Result<(), String> {
    Err("agent-bridge ainda não é suportado nesta plataforma".to_string())
}

fn parse_client_args(args: &[String]) -> Result<BridgeCommand, String> {
    let usage = "uso: agent-bridge <ready|peers|send|ask|reply> [destino|request_id] [mensagem]";
    match args.first().map(String::as_str) {
        Some("ready") if args.len() == 1 => Ok(BridgeCommand::Ready),
        Some("peers") if args.len() == 1 => Ok(BridgeCommand::Peers),
        Some("send") | Some("ask") if args.len() >= 3 => {
            let target = args[1].clone();
            let payload = args[2..].join(" ");
            if args[0] == "send" {
                Ok(BridgeCommand::Send { target, payload })
            } else {
                Ok(BridgeCommand::Ask { target, payload })
            }
        }
        Some("reply") if args.len() >= 3 => Ok(BridgeCommand::Reply {
            correlation_id: args[1].clone(),
            payload: args[2..].join(" "),
        }),
        _ => Err(usage.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bridge_commands_without_shell_interpolation() {
        assert_eq!(parse_client_args(&["ready".into()]).unwrap(), BridgeCommand::Ready);
        assert_eq!(
            parse_client_args(&["ask".into(), "Agent B".into(), "olá".into(), "mundo".into()]).unwrap(),
            BridgeCommand::Ask { target: "Agent B".into(), payload: "olá mundo".into() }
        );
        assert!(parse_client_args(&["send".into(), "B".into()]).is_err());
    }
}
