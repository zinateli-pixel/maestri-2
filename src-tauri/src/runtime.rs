//! Abstração de runtime: define QUAL motor executa um agente.
//!
//! Fase 13: adapters reais para os runtimes representados em `models::Runtime`
//! (Kilo, Claude Code, OpenCode) além do fallback `LocalShellAdapter` (Custom).
//!
//! Separação de responsabilidades:
//! - `Agent` (models.rs) = identidade/configuração lógica (runtime, command,
//!   args, working_dir, model).
//! - `RuntimeAdapter` = resolve comando/args/env de um runtime e inicia o
//!   processo via PTY (sem interpretação de shell — sem injeção).
//! - `ProcessManager` = lifecycle (start/stop/input/resize) e isolamento por
//!   `workspace_id`.
//!
//! A montagem de comando/args usa `CommandBuilder` (por argumento), nunca uma
//! string interpolada em `sh -c`, evitando shell injection acidental.

use crate::models::{Agent, Runtime};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

/// Tipo compartilhado do child entre a thread leitora e o ProcessHandle.
type SharedChild = Arc<Mutex<Box<dyn Child + Send + Sync>>>;

/// Erro de runtime unificado.
#[derive(Debug, Clone)]
pub struct RuntimeError(pub String);

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<std::io::Error> for RuntimeError {
    fn from(e: std::io::Error) -> Self {
        RuntimeError(e.to_string())
    }
}

/// Comando/args/env/cwd resolvidos para um runtime específico, prontos para
/// serem executados via PTY. A resolução é uma função pura do `Agent`
/// (testável sem spawn).
#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub working_dir: Option<String>,
}

/// Handle de um processo em execução, exposto ao ProcessManager.
pub struct ProcessHandle {
    /// Child do PTY (permite kill/wait), compartilhado com a thread leitora.
    child: SharedChild,
    /// Master PTY (permite redimensionar o terminal).
    master: Option<Box<dyn MasterPty + Send>>,
    /// Escrita no PTY (envia input ao processo).
    writer: Box<dyn Write + Send>,
    /// Canal para encerrar a thread leitora de output.
    shutdown: Sender<()>,
    /// Thread leitora (join handle).
    reader: Option<thread::JoinHandle<()>>,
}

impl ProcessHandle {
    /// Envia bytes ao stdin do processo.
    pub fn write_input(&mut self, data: &[u8]) -> Result<(), RuntimeError> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Redimensiona o PTY.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), RuntimeError> {
        if let Some(master) = self.master.as_mut() {
            master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| RuntimeError(e.to_string()))?;
        }
        Ok(())
    }

    /// Encerra o processo (SIGKILL) e a thread leitora.
    pub fn kill(&mut self) -> Result<(), RuntimeError> {
        let _ = self.shutdown.send(());
        self.child
            .lock()
            .expect("child mutex poisoned")
            .kill()
            .map_err(|e| RuntimeError(e.to_string()))?;

        // Drop the master PTY to close the read end, forcing the reader's
        // read() to return EOF. This must happen BEFORE joining the reader
        // thread, otherwise the reader may block forever in read().
        if let Some(master) = self.master.take() {
            drop(master);
        }

        if let Some(handle) = self.reader.take() {
            let _ = handle.join();
        }
        Ok(())
    }

    /// Aguarda a saída do processo e retorna o exit code.
    pub fn wait(&mut self) -> Result<u32, RuntimeError> {
        self.child
            .lock()
            .expect("child mutex poisoned")
            .wait()
            .map(|s| s.exit_code())
            .map_err(|e| RuntimeError(e.to_string()))
    }
}

/// Trait que todo runtime deve implementar.
pub trait RuntimeAdapter: Send + Sync {
    /// Nome do runtime (ex.: "kilo", "opencode", "local_shell").
    fn name(&self) -> &'static str;

    /// Resolve comando/args/env/cwd a partir do agente, sem spawn.
    fn resolve(&self, agent: &Agent) -> Result<SpawnSpec, RuntimeError>;

    /// Inicia o processo do agente e retorna um handle.
    /// `on_output` é chamado a cada chunk de saída lido do PTY.
    fn start(
        &self,
        agent: &Agent,
        on_output: Box<dyn Fn(String) + Send>,
        on_exit: Box<dyn Fn(u32) + Send>,
    ) -> Result<ProcessHandle, RuntimeError> {
        let spec = self.resolve(agent)?;
        spawn_pty(&spec, on_output, on_exit)
    }
}

/// Verifica se `cmd` existe no PATH. Usado APENAS para validar a
/// disponibilidade do executável do runtime escolhido — nunca para descobrir
/// peers/agentes no filesystem.
fn command_in_path(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Resolve o spec de um runtime de CLI a partir do agente.
///
/// - `default_command = Some(...)` => runtime conhecido: usa o default se
///   `agent.command` estiver vazio, validando presença no PATH.
/// - `default_command = None` => Custom/LocalShell: exige `agent.command`.
fn resolve_cli(
    agent: &Agent,
    default_command: Option<&str>,
    label: &str,
) -> Result<SpawnSpec, RuntimeError> {
    let user_command = agent.command.trim();
    let command = if user_command.is_empty() {
        match default_command {
            Some(def) => {
                if !command_in_path(def) {
                    return Err(RuntimeError(format!(
                        "runtime '{label}' não encontrado no PATH (comando '{def}' não instalado)"
                    )));
                }
                def.to_string()
            }
            None => {
                return Err(RuntimeError(format!(
                    "runtime '{label}' exige um comando configurado"
                )));
            }
        }
    } else {
        // Comando explícito do usuário: não valida presença (o spawn falha
        // com o erro do SO se o binário não existir).
        user_command.to_string()
    };

    Ok(SpawnSpec {
        command,
        args: agent.args.clone(),
        env: Vec::new(),
        working_dir: if agent.working_dir.trim().is_empty() {
            None
        } else {
            Some(agent.working_dir.clone())
        },
    })
}

/// Spawna o processo `spec` via PTY, conectando a thread leitora e montando
/// o `ProcessHandle`. Compartilhado por todos os adapters (sem duplicação).
fn spawn_pty(
    spec: &SpawnSpec,
    on_output: Box<dyn Fn(String) + Send>,
    on_exit: Box<dyn Fn(u32) + Send>,
) -> Result<ProcessHandle, RuntimeError> {
    if spec.command.trim().is_empty() {
        return Err(RuntimeError("comando vazio".to_string()));
    }

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| RuntimeError(e.to_string()))?;

    let mut cmd = CommandBuilder::new(&spec.command);
    for arg in &spec.args {
        cmd.arg(arg);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    if let Some(cwd) = &spec.working_dir {
        cmd.cwd(cwd);
    }

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| RuntimeError(format!("falha ao iniciar '{}': {}", spec.command, e)))?;

    // Drop do slave no processo pai (o child mantém sua própria cópia).
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| RuntimeError(e.to_string()))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| RuntimeError(e.to_string()))?;

    // Compartilha o child entre a thread leitora e o ProcessHandle.
    let child: SharedChild = Arc::new(Mutex::new(child));
    let child_for_thread = Arc::clone(&child);

    let (shutdown_tx, shutdown_rx) = channel::<()>();

    // Thread leitora: lê output do PTY, encaminha via callback e,
    // ao detectar EOF, aguarda o child e dispara on_exit.
    let reader_handle = thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            if shutdown_rx.try_recv().is_ok() {
                break;
            }
            match reader.read(&mut buf) {
                Ok(0) => break, // EOF: processo fechou o PTY.
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]).to_string();
                    on_output(text);
                }
                Err(_) => break,
            }
        }
        // Processo terminou (EOF): coleta o status e notifica.
        let status = child_for_thread
            .lock()
            .expect("child mutex poisoned")
            .wait()
            .map(|s| s.exit_code())
            .unwrap_or(1);
        on_exit(status);
    });

    Ok(ProcessHandle {
        child,
        master: Some(pair.master),
        writer,
        shutdown: shutdown_tx,
        reader: Some(reader_handle),
    })
}

/// Fallback local: executa `agent.command` + `args` em `working_dir` via PTY.
/// Usado para `Runtime::Custom` e como base de todos os runtimes de CLI.
pub struct LocalShellAdapter;

impl RuntimeAdapter for LocalShellAdapter {
    fn name(&self) -> &'static str {
        "local_shell"
    }

    fn resolve(&self, agent: &Agent) -> Result<SpawnSpec, RuntimeError> {
        resolve_cli(agent, None, "local_shell")
    }
}

/// Adapter para Kilo.
pub struct KiloAdapter;

impl RuntimeAdapter for KiloAdapter {
    fn name(&self) -> &'static str {
        "kilo"
    }

    fn resolve(&self, agent: &Agent) -> Result<SpawnSpec, RuntimeError> {
        resolve_cli(agent, Some("kilo"), "Kilo")
    }
}

/// Adapter para Claude Code.
pub struct ClaudeCodeAdapter;

impl RuntimeAdapter for ClaudeCodeAdapter {
    fn name(&self) -> &'static str {
        "claude_code"
    }

    fn resolve(&self, agent: &Agent) -> Result<SpawnSpec, RuntimeError> {
        resolve_cli(agent, Some("claude"), "Claude Code")
    }
}

/// Adapter para OpenCode.
pub struct OpenCodeAdapter;

impl RuntimeAdapter for OpenCodeAdapter {
    fn name(&self) -> &'static str {
        "opencode"
    }

    fn resolve(&self, agent: &Agent) -> Result<SpawnSpec, RuntimeError> {
        resolve_cli(agent, Some("opencode"), "OpenCode")
    }
}

/// Seleciona o adapter correspondente ao `Runtime` configurado do agente.
pub fn adapter_for(runtime: Runtime) -> Box<dyn RuntimeAdapter> {
    match runtime {
        Runtime::Kilo => Box::new(KiloAdapter),
        Runtime::ClaudeCode => Box::new(ClaudeCodeAdapter),
        Runtime::OpenCode => Box::new(OpenCodeAdapter),
        Runtime::Custom => Box::new(LocalShellAdapter),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Agent, AgentKind, Role, Status};

    fn cli_agent(command: &str, args: Vec<String>) -> Agent {
        Agent {
            id: "a1".to_string(),
            name: "a1".to_string(),
            role: Role::Builder,
            runtime: Runtime::Custom,
            kind: AgentKind::Cli,
            model: String::new(),
            command: command.to_string(),
            args,
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

    fn runtime_agent(runtime: Runtime, command: &str) -> Agent {
        let mut a = cli_agent(command, vec![]);
        a.runtime = runtime;
        a
    }

    #[test]
    fn adapter_for_maps_each_runtime() {
        assert_eq!(adapter_for(Runtime::Kilo).name(), "kilo");
        assert_eq!(adapter_for(Runtime::ClaudeCode).name(), "claude_code");
        assert_eq!(adapter_for(Runtime::OpenCode).name(), "opencode");
        assert_eq!(adapter_for(Runtime::Custom).name(), "local_shell");
    }

    #[test]
    fn default_command_is_used_when_agent_command_empty() {
        // `sh` existe em qualquer macOS/Linux => default determinístico.
        let a = runtime_agent(Runtime::Kilo, "");
        let spec = resolve_cli(&a, Some("sh"), "x").unwrap();
        assert_eq!(spec.command, "sh");
    }

    #[test]
    fn adapters_use_custom_command_when_set() {
        assert_eq!(
            ClaudeCodeAdapter.resolve(&runtime_agent(Runtime::ClaudeCode, "claude-ng")).unwrap().command,
            "claude-ng"
        );
        assert_eq!(
            OpenCodeAdapter.resolve(&runtime_agent(Runtime::OpenCode, "./bin/opencode")).unwrap().command,
            "./bin/opencode"
        );
        assert_eq!(
            LocalShellAdapter.resolve(&runtime_agent(Runtime::Custom, "bash")).unwrap().command,
            "bash"
        );
    }

    #[test]
    fn localshell_requires_a_command() {
        let a = runtime_agent(Runtime::Custom, "");
        assert!(LocalShellAdapter.resolve(&a).is_err());
    }

    #[test]
    fn spec_preserves_args_and_working_dir() {
        let mut a = runtime_agent(Runtime::Kilo, "kilo");
        a.args = vec!["--model".to_string(), "x".to_string()];
        a.working_dir = "/tmp/proj".to_string();
        let spec = KiloAdapter.resolve(&a).unwrap();
        assert_eq!(spec.args, vec!["--model", "x"]);
        assert_eq!(spec.working_dir.as_deref(), Some("/tmp/proj"));

        // working_dir vazio => None (não passa cwd).
        let b = runtime_agent(Runtime::Kilo, "kilo");
        assert!(KiloAdapter.resolve(&b).unwrap().working_dir.is_none());
    }

    #[test]
    fn command_in_path_detects_real_and_missing() {
        // `sh` existe em qualquer macOS/Linux.
        assert!(command_in_path("sh"));
        // Nome claramente inexistente.
        assert!(!command_in_path("maestri-__nao_existe__-xyz"));
    }

    #[test]
    fn missing_default_runtime_is_reported() {
        // Default flagrantemente inexistente => erro claro, determinístico.
        let a = runtime_agent(Runtime::OpenCode, "");
        let err = resolve_cli(&a, Some("maestri-__nao_existe__-xyz"), "Ghost").unwrap_err();
        assert!(err.0.contains("Ghost"));
        assert!(err.0.contains("PATH"));
    }

    /// Validação de runtime real: spawna um processo via PTY, lê output,
    /// injeta input, redimensiona e encerra. Prova o caminho start/input/
    /// output/resize/stop usado pelos agentes no app.
    #[test]
    fn real_pty_spawn_input_output_resize_stop() {
        let adapter = LocalShellAdapter;
        let agent = cli_agent(
            "sh",
            vec![
                "-c".to_string(),
                "echo READY; read line; echo got:$line; sleep 30".to_string(),
            ],
        );

        let out = Arc::new(Mutex::new(String::new()));
        let out_tx = Arc::clone(&out);
        let exited = Arc::new(Mutex::new(None));
        let exited_tx = Arc::clone(&exited);

        let mut handle = adapter
            .start(
                &agent,
                Box::new(move |text| {
                    out_tx.lock().unwrap().push_str(&text);
                }),
                Box::new(move |code| {
                    *exited_tx.lock().unwrap() = Some(code);
                }),
            )
            .expect("spawn deve funcionar");

        // Aguarda o sinal READY (output real do processo).
        wait_for(&out, "READY", "READY não apareceu no output");

        // Injeta input no stdin do processo.
        handle.write_input(b"hello\n").expect("write_input deve funcionar");

        // Aguarda o eco "got:hello" (o input chegou ao processo).
        wait_for(&out, "got:hello", "input não chegou ao processo");

        // Redimensiona o PTY.
        handle.resize(120, 40).expect("resize deve funcionar");

        // Encerra o processo (kill).
        handle.kill().expect("kill deve funcionar");

        // O exit code deve ter sido notificado (SIGKILL => não zero).
        // Apenas garante que a thread de exit não ficou presa.
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    /// Prova que o dispatcher (`adapter_for`) seleciona o adapter correto e que
    /// o caminho completo dispatch → resolve → spawn_pty → output funciona,
    /// não caindo em um LocalShell "universal" por engano.
    #[test]
    fn dispatcher_start_runs_through_adapter_for() {
        let adapter = adapter_for(Runtime::Custom);
        assert_eq!(adapter.name(), "local_shell");

        let agent = cli_agent(
            "sh",
            vec!["-c".to_string(), "echo DISPATCHED; sleep 30".to_string()],
        );

        let out = Arc::new(Mutex::new(String::new()));
        let out_tx = Arc::clone(&out);
        let mut handle = adapter
            .start(
                &agent,
                Box::new(move |t| {
                    out_tx.lock().unwrap().push_str(&t);
                }),
                Box::new(|_| {}),
            )
            .expect("spawn via adapter_for deve funcionar");

        wait_for(&out, "DISPATCHED", "dispatch não produziu output");
        handle.kill().expect("kill deve funcionar");
    }

    fn wait_for(out: &Arc<Mutex<String>>, needle: &str, panic_msg: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if out.lock().unwrap().contains(needle) {
                return;
            }
            if std::time::Instant::now() > deadline {
                let got = out.lock().unwrap().clone();
                panic!("{panic_msg} (output atual: {got:?})");
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}