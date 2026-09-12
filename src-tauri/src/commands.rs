use crate::models::{Agent, AgentKind, Edge, EdgeType, Project, ProjectWorkspaceInput, CreateProjectInput, RenameProjectInput, Role, Runtime, Status, Viewport, WorkspaceListItem, WorkspaceSettings, WorkspaceState, WorkspaceMetadata, CreateWorkspaceInput, RenameWorkspaceInput, UpdateWorkspaceSettingsInput, UpdateViewportInput, validate_edge};
use crate::state::AppState;
use crate::workflow::{WorkflowDefinition, WorkflowExecution};
use serde::Deserialize;
use tauri::{AppHandle, Manager, State};
use std::sync::Arc;

/// Retorna o estado completo do workspace (agentes + versão).
#[tauri::command]
pub fn get_workspace_state(state: State<'_, AppState>) -> WorkspaceState {
    let guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    guard.clone()
}

/// Retorna apenas a lista de agentes.
#[tauri::command]
pub fn get_agents(state: State<'_, AppState>) -> Vec<Agent> {
    let guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    guard.agents.clone()
}

/// Payload de criação de agente (sem id — o backend gera).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateAgentInput {
    pub name: String,
    pub role: Role,
    pub runtime: Runtime,
    pub model: String,
    pub command: String,
    pub args: Vec<String>,
    pub working_dir: String,
    #[serde(default)]
    pub auto_start: bool,
}

/// Payload de atualização de agente (id obrigatório).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UpdateAgentInput {
    pub id: String,
    pub name: String,
    pub role: Role,
    pub runtime: Runtime,
    pub model: String,
    pub command: String,
    pub args: Vec<String>,
    pub working_dir: String,
    #[serde(default)]
    pub collapsed: Option<bool>,
    #[serde(default)]
    pub locked: Option<bool>,
    #[serde(default)]
    pub accent: Option<String>,
    #[serde(default)]
    pub auto_start: Option<bool>,
}

/// Cria um agente, gera o id no backend, persiste e retorna o agente criado.
#[tauri::command]
pub fn create_agent(
    state: State<'_, AppState>,
    input: CreateAgentInput,
) -> Result<Agent, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("nome é obrigatório".to_string());
    }

    let id = generate_id(&state);

    let agent = Agent {
        id,
        name,
        role: input.role,
        runtime: input.runtime,
        kind: AgentKind::Cli,
        model: input.model,
        command: input.command,
        args: input.args,
        working_dir: input.working_dir,
        status: Status::Idle,
        x: 0.5,
        y: 0.5,
        width: None,
        height: None,
        collapsed: false,
        locked: false,
        accent: None,
        auto_start: input.auto_start,
    };

    {
        let mut guard = state
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        guard.agents.push(agent.clone());
    }

    state.persist()?;
    Ok(agent)
}

/// Atualiza um agente existente (valida id e nome) e persiste.
#[tauri::command]
pub fn update_agent(
    state: State<'_, AppState>,
    input: UpdateAgentInput,
) -> Result<Agent, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("nome é obrigatório".to_string());
    }

    let mut guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");

    let agent = guard
        .agents
        .iter_mut()
        .find(|a| a.id == input.id)
        .ok_or_else(|| "agente não encontrado".to_string())?;

    agent.name = name;
    agent.role = input.role;
    agent.runtime = input.runtime;
    agent.model = input.model;
    agent.command = input.command;
    agent.args = input.args;
    agent.working_dir = input.working_dir;
    if let Some(collapsed) = input.collapsed {
        agent.collapsed = collapsed;
    }
    if let Some(locked) = input.locked {
        agent.locked = locked;
    }
    if input.accent.is_some() {
        agent.accent = input.accent;
    }
    if let Some(auto_start) = input.auto_start {
        agent.auto_start = auto_start;
    }

    let updated = agent.clone();
    drop(guard);

    state.touch();
    state.persist()?;
    Ok(updated)
}

/// Remove um agente pelo id e persiste.
#[tauri::command]
pub fn delete_agent(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    // Para o processo antes de remover o agente — caso contrário o
    // processo (PTY + thread leitora) ficaria órfão, rodando sem dono.
    let _ = state.processes.stop(&app, &id);

    {
        let mut guard = state
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        let before = guard.agents.len();
        guard.agents.retain(|a| a.id != id);
        if guard.agents.len() == before {
            return Err("agente não encontrado".to_string());
        }
        // Remove edges conectadas ao agente deletado.
        guard.edges.retain(|e| e.source != id && e.target != id);
    }

    state.persist()
}

/// Atualiza a posição de um agente no canvas e persiste.
#[tauri::command]
pub fn update_agent_position(
    state: State<'_, AppState>,
    id: String,
    x: f64,
    y: f64,
) -> Result<(), String> {
    {
        let mut guard = state
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        let agent = guard
            .agents
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| "agente não encontrado".to_string())?;
        agent.x = x;
        agent.y = y;
    }

    state.touch();
    state.persist()
}

/// Atualiza posição + tamanho de um agente (usado no resize do node).
#[tauri::command]
pub fn update_agent_geometry(
    state: State<'_, AppState>,
    id: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    {
        let mut guard = state
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        let agent = guard
            .agents
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| "agente não encontrado".to_string())?;
        agent.x = x;
        agent.y = y;
        agent.width = Some(width.max(240.0));
        agent.height = Some(height.max(120.0));
    }

    state.touch();
    state.persist()
}

/// Duplica um agente (nova id, posição com offset, status Idle).
/// Não duplica processo/PTY — apenas a configuração.
#[tauri::command]
pub fn duplicate_agent(state: State<'_, AppState>, id: String) -> Result<Agent, String> {
    let source = {
        let guard = state
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        guard
            .agents
            .iter()
            .find(|a| a.id == id)
            .cloned()
            .ok_or_else(|| "agente não encontrado".to_string())?
    };

    let new_id = generate_id(&state);

    let agent = Agent {
        id: new_id,
        name: format!("{} copy", source.name),
        role: source.role,
        runtime: source.runtime,
        kind: source.kind,
        model: source.model,
        command: source.command,
        args: source.args,
        working_dir: source.working_dir,
        status: Status::Idle,
        x: (source.x + 0.05).min(0.95),
        y: (source.y + 0.08).min(0.95),
        width: source.width,
        height: source.height,
        collapsed: false,
        locked: false,
        accent: source.accent,
        auto_start: false, // Cópias não iniciam automaticamente
    };

    {
        let mut guard = state
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        guard.agents.push(agent.clone());
    }

    state.touch();
    state.persist()?;
    Ok(agent)
}

/// Reinicia o processo de um agente (stop tolerante + start).
#[tauri::command]
pub fn restart_agent(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    // Para se estiver rodando (ignora erro se não estiver).
    let _ = state.processes.stop(&app, &id);

    let agent = {
        let guard = state
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        guard
            .agents
            .iter()
            .find(|a| a.id == id)
            .cloned()
            .ok_or_else(|| "agente não encontrado".to_string())?
    };

    if agent.command.trim().is_empty() {
        return Err("comando vazio — configure o comando do agente".to_string());
    }

    set_status(&state, &id, Status::Starting)?;
    match state.processes.start(&app, &agent) {
        Ok(()) => {
            set_status(&state, &id, Status::Running)?;
            Ok(())
        }
        Err(e) => {
            set_status(&state, &id, Status::Failed)?;
            Err(e.to_string())
        }
    }
}

/// Atualiza o processo de um agente (stop + cleanup + start atômico).
/// Operação atômica: para processo antigo, limpa recursos, cria novo PTY e inicia runtime.
#[tauri::command]
pub fn refresh_agent(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    eprintln!("[REFRESH] begin agent_id={}", id);
    // 1. Obtém o agente (precisamos do comando antes de travar o processo)
    let agent = {
        let guard = state
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        guard
            .agents
            .iter()
            .find(|a| a.id == id)
            .cloned()
            .ok_or_else(|| "agente não encontrado".to_string())?
    };
    eprintln!("[REFRESH] agent found: {} command={}", agent.name, agent.command);

    if agent.command.trim().is_empty() {
        return Err("comando vazio — configure o comando do agente".to_string());
    }

    // 2. Para processo existente se houver (cleanup completo)
    eprintln!("[REFRESH] stopping existing process...");
    let _ = state.processes.stop(&app, &id);
    eprintln!("[REFRESH] stop returned");

    // 3. Atualiza status para Starting
    eprintln!("[REFRESH] setting status Starting...");
    set_status(&state, &id, Status::Starting)?;
    eprintln!("[REFRESH] status set to Starting");

    // 4. Inicia novo processo (cria PTY fresco, spawn runtime, registra reader)
    eprintln!("[REFRESH] starting new process...");
    match state.processes.start(&app, &agent) {
        Ok(()) => {
            eprintln!("[REFRESH] start ok, setting status Running...");
            set_status(&state, &id, Status::Running)?;
            eprintln!("[REFRESH] refresh complete");
            Ok(())
        }
        Err(e) => {
            eprintln!("[REFRESH] start failed: {}", e);
            set_status(&state, &id, Status::Failed)?;
            Err(e.to_string())
        }
    }
}

/// Informação de um runtime detectável localmente.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RuntimeInfo {
    pub id: String,
    pub name: String,
    pub command: String,
    pub available: bool,
    pub capabilities: Vec<String>,
}

/// Detecta executáveis conhecidos no sistema (sem instalar nada).
#[tauri::command]
pub fn detect_runtimes() -> Vec<RuntimeInfo> {
    let candidates = [
        ("kilo", "Kilo", "kilo"),
        ("claude_code", "Claude Code", "claude"),
        ("opencode", "OpenCode", "opencode"),
        ("codex", "Codex", "codex"),
        ("ollama", "Ollama", "ollama"),
        ("zsh", "Shell (zsh)", "zsh"),
        ("bash", "Shell (bash)", "bash"),
    ];

    candidates
        .iter()
        .map(|(id, name, cmd)| RuntimeInfo {
            id: id.to_string(),
            name: name.to_string(),
            command: cmd.to_string(),
            available: which(cmd),
            capabilities: vec![
                "terminal".to_string(),
                "interactive".to_string(),
                "resize".to_string(),
                "restart".to_string(),
            ],
        })
        .collect()
}

/// Verifica se um executável existe no PATH (via `which`).
fn which(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Exporta o workspace atual como JSON (string).
#[tauri::command]
pub fn export_workspace(state: State<'_, AppState>) -> Result<String, String> {
    let guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    serde_json::to_string_pretty(&*guard).map_err(|e| format!("falha ao serializar: {e}"))
}

/// Importa um workspace a partir de JSON, validando o schema.
/// Substitui o estado atual (o frontend deve pedir confirmação antes).
#[tauri::command]
pub fn import_workspace(
    state: State<'_, AppState>,
    json: String,
) -> Result<WorkspaceState, String> {
    let parsed: WorkspaceState = serde_json::from_str(&json)
        .map_err(|e| format!("arquivo inválido: {e}"))?;

    // Validação mínima de schema.
    for agent in &parsed.agents {
        if agent.id.trim().is_empty() {
            return Err("workspace inválido: agente sem id".to_string());
        }
        if agent.name.trim().is_empty() {
            return Err("workspace inválido: agente sem nome".to_string());
        }
    }

    // Para processos do workspace substituído e atualiza o id corrente
    // (o JSON importado pode ter um metadata.id diferente).
    state.stop_all_processes();
    state.set_current_workspace(parsed.clone());

    state.persist()?;
    Ok(parsed)
}

/// Substitui o estado do workspace (usado por undo/redo).
#[tauri::command]
pub fn restore_workspace(
    state: State<'_, AppState>,
    snapshot: WorkspaceState,
) -> Result<(), String> {
    {
        let mut guard = state
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        *guard = snapshot;
    }
    state.persist()
}

/// Cria um backup timestamped do workspace atual.
/// Retorna o caminho do arquivo de backup criado.
#[tauri::command]
pub fn backup_workspace(state: State<'_, AppState>) -> Result<String, String> {
    let guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    state.persistence.backup_workspace(&guard)
}

/// Lista os backups disponíveis para o workspace atual.
#[tauri::command]
pub fn list_backups(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    state.persistence.list_backups(&guard.metadata.id)
}

/// Comando mínimo de health-check (ping).
#[tauri::command]
pub fn ping() -> String {
    "pong".to_string()
}

/// Inicia o processo real de um agente (Fase 3A: local shell via PTY).
#[tauri::command]
pub fn start_agent(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    // Valida o agente e seu comando.
    let agent = {
        let guard = state
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        guard
            .agents
            .iter()
            .find(|a| a.id == id)
            .cloned()
            .ok_or_else(|| "agente não encontrado".to_string())?
    };

    if agent.command.trim().is_empty() {
        return Err("comando vazio — configure o comando do agente".to_string());
    }

    // Marca status como Starting antes de iniciar.
    set_status(&state, &id, Status::Starting)?;

    // Inicia o processo real.
    let result = state.processes.start(&app, &agent);

    match result {
        Ok(()) => {
            set_status(&state, &id, Status::Running)?;
            Ok(())
        }
        Err(e) => {
            set_status(&state, &id, Status::Failed)?;
            Err(e.to_string())
        }
    }
}

/// Para o processo real de um agente.
#[tauri::command]
pub fn stop_agent(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state
        .processes
        .stop(&app, &id)
        .map_err(|e| e.to_string())?;
    set_status(&state, &id, Status::Stopped)?;
    Ok(())
}

/// Envia input (stdin) ao processo de um agente.
#[tauri::command]
pub fn send_agent_input(
    state: State<'_, AppState>,
    id: String,
    input: String,
) -> Result<(), String> {
    let workspace_id = {
        let guard = state
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        guard.metadata.id.clone()
    };
    state
        .processes
        .send_input_in(&workspace_id, &id, &input)
        .map_err(|e| e.to_string())
}

/// Redimensiona o PTY de um agente.
#[tauri::command]
pub fn resize_agent(
    state: State<'_, AppState>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let workspace_id = {
        let guard = state
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        guard.metadata.id.clone()
    };
    state
        .processes
        .resize_in(&workspace_id, &id, cols, rows)
        .map_err(|e| e.to_string())
}

/// Atualiza o status de um agente no workspace e persiste.
fn set_status(state: &AppState, id: &str, status: Status) -> Result<(), String> {
    {
        let mut guard = state
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        let agent = guard
            .agents
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| "agente não encontrado".to_string())?;
        agent.status = status;
    }
    state.persist()
}

/// Payload de criação de edge (sem id — o backend gera).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateEdgeInput {
    pub source: String,
    pub target: String,
    pub source_handle: Option<String>,
    pub target_handle: Option<String>,
}

/// Cria uma conexão entre dois agentes, persiste e retorna a edge criada.
#[tauri::command]
pub fn add_edge(
    state: State<'_, AppState>,
    input: CreateEdgeInput,
) -> Result<Edge, String> {
    let source = input.source.clone();
    let target = input.target.clone();

    let mut guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");

    // Valida: self-loop, existência de origem/destino e duplicata.
    validate_edge(&guard.agents, &guard.edges, &input.source, &input.target)?;

    let edge = Edge {
        id: format!("edge-{}", timestamp_millis()),
        source: input.source,
        target: input.target,
        source_handle: input.source_handle,
        target_handle: input.target_handle,
        edge_type: EdgeType::Message,
        label: None,
    };

    guard.edges.push(edge.clone());
    drop(guard);

    state.persist()?;

    // A topologia do canvas é a fonte de verdade: notifica os agentes em
    // execução sobre a nova conexão imediatamente.
    state.announce_peers_to(&source);
    state.announce_peers_to(&target);

    Ok(edge)
}

/// Remove uma conexão pelo id e persiste.
#[tauri::command]
pub fn remove_edge(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let (source, target) = {
        let mut guard = state
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        let edge = guard
            .edges
            .iter()
            .find(|e| e.id == id)
            .cloned()
            .ok_or_else(|| "conexão não encontrada".to_string())?;
        let s = edge.source;
        let t = edge.target;
        guard.edges.retain(|e| e.id != id);
        (s, t)
    };
    state.persist()?;

    // Notifica os agentes afetados da remoção da conexão.
    state.announce_peers_to(&source);
    state.announce_peers_to(&target);

    Ok(())
}

/// Gera um id único baseado em contador + timestamp.
fn generate_id(state: &AppState) -> String {
    let guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    let n = guard.agents.len();
    format!("agent-{}-{}", n + 1, timestamp_millis())
}

fn timestamp_millis() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

// ============================================================================
// WORKSPACE MANAGEMENT COMMANDS
// ============================================================================

/// Lista todos os workspaces disponíveis.
#[tauri::command]
pub fn list_workspaces(state: State<'_, AppState>) -> Result<Vec<WorkspaceListItem>, String> {
    state.persistence.list_workspaces()
}

/// Cria um novo workspace.
#[tauri::command]
pub fn create_workspace(
    state: State<'_, AppState>,
    input: CreateWorkspaceInput,
) -> Result<WorkspaceState, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("nome é obrigatório".to_string());
    }

    let workspace = WorkspaceState::new(name);
    state.persistence.save_workspace(&workspace)?;

    // Para todos os processos e reseta status antes de trocar de workspace
    // (não pode misturar agentes entre workspaces).
    state.stop_all_processes();

    // Define como workspace atual
    state.set_current_workspace(workspace.clone());

    Ok(workspace)
}

/// Carrega um workspace específico.
#[tauri::command]
pub fn load_workspace(
    state: State<'_, AppState>,
    id: String,
) -> Result<WorkspaceState, String> {
    let workspace = state.load_workspace(&id)?;

    // Para todos os processos e reseta status antes de trocar de workspace
    // (não pode misturar agentes entre workspaces).
    state.stop_all_processes();

    state.set_current_workspace(workspace.clone());
    Ok(workspace)
}

/// Renomeia um workspace.
#[tauri::command]
pub fn rename_workspace(
    state: State<'_, AppState>,
    input: RenameWorkspaceInput,
) -> Result<WorkspaceState, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("nome é obrigatório".to_string());
    }

    let mut guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    
    if guard.metadata.id != input.id {
        return Err("workspace não encontrado".to_string());
    }
    
    guard.metadata.name = name;
    guard.touch();
    drop(guard);
    
    state.persist()?;
    
    // Retorna o workspace atualizado
    let guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    Ok(guard.clone())
}

/// Deleta um workspace.
#[tauri::command]
pub fn delete_workspace(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    // Não permite deletar se for o único workspace
    let workspaces = state.persistence.list_workspaces()?;
    if workspaces.len() <= 1 {
        return Err("não é possível deletar o único workspace".to_string());
    }
    
    // Se for o workspace atual, para os processos e carrega outro
    let current_id = state.current_workspace_id();
    if current_id.as_ref().map(|s| s.as_str()) == Some(id.as_str()) {
        // Para todos os processos do workspace que será deletado
        // (evita processos órfãos de um workspace que deixa de existir).
        state.stop_all_processes();
        let workspaces = state.persistence.list_workspaces()?;
        if let Some(next) = workspaces.iter().find(|w| w.id != id) {
            let next_ws = state.load_workspace(&next.id)?;
            state.set_current_workspace(next_ws);
        }
    }
    
    state.persistence.delete_workspace(&id)
}

/// Duplica um workspace.
#[tauri::command]
pub fn duplicate_workspace(
    state: State<'_, AppState>,
    id: String,
) -> Result<WorkspaceState, String> {
    let workspace = state.load_workspace(&id)?;
    
    let mut new_workspace = workspace.clone();
    new_workspace.metadata = WorkspaceMetadata {
        id: format!("workspace-{}", timestamp_millis()),
        name: format!("{} (copy)", workspace.metadata.name),
        created_at: timestamp_millis() as u64,
        updated_at: timestamp_millis() as u64,
        last_opened_at: timestamp_millis() as u64,
    };
    
    // Reset status dos agentes
    for agent in &mut new_workspace.agents {
        agent.status = Status::Idle;
    }
    
    state.persistence.save_workspace(&new_workspace)?;
    Ok(new_workspace)
}

/// Atualiza as configurações do workspace.
#[tauri::command]
pub fn update_workspace_settings(
    state: State<'_, AppState>,
    input: UpdateWorkspaceSettingsInput,
) -> Result<WorkspaceState, String> {
    let mut guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    
    if guard.metadata.id != input.id {
        return Err("workspace não encontrado".to_string());
    }
    
    guard.settings = input.settings;
    guard.touch();
    drop(guard);
    
    state.persist()?;
    
    let guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    Ok(guard.clone())
}

/// Atualiza o viewport do workspace.
#[tauri::command]
pub fn update_viewport(
    state: State<'_, AppState>,
    input: UpdateViewportInput,
) -> Result<(), String> {
    let mut guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    
    if guard.metadata.id != input.id {
        return Err("workspace não encontrado".to_string());
    }
    
    guard.viewport = input.viewport;
    guard.touch();
    drop(guard);
    
    state.persist()
}

/// Lista workspaces disponíveis (para o seletor).
#[tauri::command]
pub fn get_workspace_list(state: State<'_, AppState>) -> Result<Vec<WorkspaceListItem>, String> {
    state.persistence.list_workspaces()
}

// ============================================================================
// PROJECT COMMANDS (Fase 12)
// ============================================================================

/// Lista todos os projetos.
#[tauri::command]
pub fn list_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    state.persistence.list_projects()
}

/// Cria um novo projeto (vazio, sem workspaces).
#[tauri::command]
pub fn create_project(
    state: State<'_, AppState>,
    input: CreateProjectInput,
) -> Result<Project, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("nome é obrigatório".to_string());
    }
    let now = timestamp_millis() as u64;
    let project = Project::new(name, now);
    state.persistence.save_project(&project)?;
    Ok(project)
}

/// Renomeia um projeto.
#[tauri::command]
pub fn rename_project(
    state: State<'_, AppState>,
    input: RenameProjectInput,
) -> Result<Project, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("nome é obrigatório".to_string());
    }
    let mut project = state
        .persistence
        .load_project(&input.id)?
        .ok_or_else(|| "projeto não encontrado".to_string())?;
    project.name = name;
    project.touch(timestamp_millis() as u64);
    state.persistence.save_project(&project)?;
    Ok(project)
}

/// Deleta um projeto (não apaga os workspaces associados).
#[tauri::command]
pub fn delete_project(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.persistence.delete_project(&id)
}

/// Associa um workspace a um projeto (idempotente).
#[tauri::command]
pub fn add_workspace_to_project(
    state: State<'_, AppState>,
    input: ProjectWorkspaceInput,
) -> Result<Project, String> {
    // Garante que o workspace existe antes de associar.
    if state.persistence.load_workspace(&input.workspace_id)?.is_none() {
        return Err("workspace não encontrado".to_string());
    }
    let mut project = state
        .persistence
        .load_project(&input.project_id)?
        .ok_or_else(|| "projeto não encontrado".to_string())?;
    project.add_workspace(&input.workspace_id);
    project.touch(timestamp_millis() as u64);
    state.persistence.save_project(&project)?;
    Ok(project)
}

/// Desassocia um workspace de um projeto.
#[tauri::command]
pub fn remove_workspace_from_project(
    state: State<'_, AppState>,
    input: ProjectWorkspaceInput,
) -> Result<Project, String> {
    let mut project = state
        .persistence
        .load_project(&input.project_id)?
        .ok_or_else(|| "projeto não encontrado".to_string())?;
    project.remove_workspace(&input.workspace_id);
    project.touch(timestamp_millis() as u64);
    state.persistence.save_project(&project)?;
    Ok(project)
}

// ==================== WORKFLOW COMMANDS ====================

/// Lista todos os workflows.
#[tauri::command]
pub fn list_workflows(state: State<'_, AppState>) -> Result<Vec<WorkflowDefinition>, String> {
    Ok(state.list_workflows())
}

/// Obtém um workflow por ID.
#[tauri::command]
pub fn get_workflow(state: State<'_, AppState>, id: String) -> Result<Option<WorkflowDefinition>, String> {
    Ok(state.get_workflow(&id))
}

/// Cria um novo workflow.
#[tauri::command]
pub fn create_workflow(state: State<'_, AppState>, mut workflow: WorkflowDefinition) -> Result<WorkflowDefinition, String> {
    let wf = state.create_workflow(workflow);
    state.persistence.save_workflow(&wf)?;
    Ok(wf)
}

/// Atualiza um workflow existente.
#[tauri::command]
pub fn update_workflow(state: State<'_, AppState>, workflow: WorkflowDefinition) -> Result<WorkflowDefinition, String> {
    let wf = state.update_workflow(workflow)?;
    state.persistence.save_workflow(&wf)?;
    Ok(wf)
}

/// Deleta um workflow.
#[tauri::command]
pub fn delete_workflow(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.delete_workflow(&id)?;
    state.persistence.delete_workflow(&id)
}

/// Lista todas as execuções.
#[tauri::command]
pub fn list_executions(state: State<'_, AppState>) -> Result<Vec<WorkflowExecution>, String> {
    Ok(state.list_executions())
}

/// Obtém uma execução por ID.
#[tauri::command]
pub fn get_execution(state: State<'_, AppState>, id: String) -> Result<Option<WorkflowExecution>, String> {
    Ok(state.get_execution(&id))
}

/// Inicia execução de um workflow.
#[tauri::command]
pub fn start_workflow_execution(
    app: AppHandle,
    state: State<'_, AppState>,
    workflow_id: String,
) -> Result<WorkflowExecution, String> {
    let workflow = state.get_workflow(&workflow_id)
        .ok_or_else(|| "workflow não encontrado".to_string())?;

    let agents = {
        let guard = state.workspace.lock().unwrap();
        guard.agents.clone()
    };

    // Valida se todos os agents referenciados existem
    workflow.validate(&agents)?;

    let executor = Arc::new(crate::workflow::ProcessNodeExecutor::new(Arc::new(state.processes.clone())));
    let app_for_engine = app.clone();
    let engine = crate::workflow::WorkflowEngine::new(app_for_engine, executor);

    // Persist callback: usa o AppHandle para acessar o estado gerenciado
    let execution = engine.start_execution(&workflow, &agents, move |app_opt, exec| {
        if let Some(app) = app_opt {
            let state: State<'_, AppState> = app.state();
            state.upsert_execution(exec.clone());
            let _ = state.persistence.save_execution(exec);
        }
    })?;

    Ok(execution)
}

/// Cancela uma execução em andamento.
#[tauri::command]
pub fn cancel_workflow_execution(
    state: State<'_, AppState>,
    execution_id: String,
) -> Result<(), String> {
    // Marca flag de cancelamento (o engine verifica periodicamente)
    // Nota: o engine real roda em thread separada, então isso é best-effort
    // Para implementação completa, precisaríamos de um registry de engines ativos
    // Por enquanto, apenas atualiza o status na execução persistida
    if let Some(mut exec) = state.get_execution(&execution_id) {
        if exec.status == crate::workflow::ExecutionStatus::Running || exec.status == crate::workflow::ExecutionStatus::Pending {
            exec.status = crate::workflow::ExecutionStatus::Cancelled;
            exec.finished_at = Some(crate::workflow::now_millis());
            exec.error = Some("cancelled by user".to_string());
            state.upsert_execution(exec.clone());
            state.persistence.save_execution(&exec)?;
        }
    }
    Ok(())
}

/// Deleta uma execução.
#[tauri::command]
pub fn delete_execution(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.persistence.delete_execution(&id)
}