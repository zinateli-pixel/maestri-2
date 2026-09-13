use crate::events::WorkflowEvent;
use crate::memory::MemoryEntry;
use crate::models::{Agent, Edge, Project, Viewport, WorkspaceListItem, WorkspaceMetadata, WorkspaceSettings, WorkspaceState};
use crate::permissions::PermissionGrant;
use crate::workflow::{WorkflowDefinition, WorkflowExecution};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Formato legado (v1) de workspace único — usado apenas para migração.
#[derive(serde::Deserialize)]
struct LegacyWorkspace {
    #[serde(default)]
    agents: Vec<Agent>,
    #[serde(default)]
    edges: Vec<Edge>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    updated_at: u64,
    #[serde(default)]
    version: String,
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Diretório onde os workspaces são salvos.
fn workspaces_dir(app_data_dir: &PathBuf) -> PathBuf {
    app_data_dir.join("workspaces")
}

/// Caminho do arquivo de índice de workspaces.
fn index_path(app_data_dir: &PathBuf) -> PathBuf {
    workspaces_dir(app_data_dir).join("index.json")
}

/// Caminho do arquivo de um workspace específico.
fn workspace_path(app_data_dir: &PathBuf, workspace_id: &str) -> PathBuf {
    workspaces_dir(app_data_dir).join(format!("{}.json", workspace_id))
}

/// Serviço de persistência para múltiplos workspaces.
pub struct Persistence {
    app_data_dir: PathBuf,
}

impl Persistence {
    /// Cria o serviço apontando para o diretório de dados da aplicação.
    pub fn new(app_data_dir: PathBuf) -> Self {
        Self { app_data_dir }
    }

    /// Inicializa o diretório de workspaces e o índice se não existirem.
    fn ensure_initialized(&self) -> Result<(), String> {
        let dir = workspaces_dir(&self.app_data_dir);
        fs::create_dir_all(&dir).map_err(|e| format!("falha ao criar diretório de workspaces: {e}"))?;
        
        let index_file = index_path(&self.app_data_dir);
        if !index_file.exists() {
            let initial_index: Vec<crate::models::WorkspaceListItem> = Vec::new();
            let raw = serde_json::to_string_pretty(&initial_index)
                .map_err(|e| format!("falha ao serializar índice inicial: {e}"))?;
            fs::write(&index_file, raw).map_err(|e| format!("falha ao gravar índice inicial: {e}"))?;
        }
        Ok(())
    }

    /// Carrega a lista de workspaces disponíveis.
    pub fn list_workspaces(&self) -> Result<Vec<crate::models::WorkspaceListItem>, String> {
        self.ensure_initialized()?;
        let raw = fs::read_to_string(index_path(&self.app_data_dir))
            .map_err(|e| format!("falha ao ler índice: {e}"))?;
        serde_json::from_str(&raw).map_err(|e| format!("falha ao desserializar índice: {e}"))
    }

    /// Carrega um workspace específico por ID.
    pub fn load_workspace(&self, workspace_id: &str) -> Result<Option<WorkspaceState>, String> {
        self.ensure_initialized()?;
        let path = workspace_path(&self.app_data_dir, workspace_id);
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path).map_err(|e| format!("falha ao ler workspace: {e}"))?;
        let workspace = serde_json::from_str(&raw).map_err(|e| format!("falha ao desserializar workspace: {e}"))?;
        Ok(Some(workspace))
    }

    /// Salva um workspace (cria ou atualiza).
    pub fn save_workspace(&self, workspace: &WorkspaceState) -> Result<(), String> {
        self.ensure_initialized()?;
        
        // Salva o arquivo do workspace
        let path = workspace_path(&self.app_data_dir, &workspace.metadata.id);
        let raw = serde_json::to_string_pretty(workspace)
            .map_err(|e| format!("falha ao serializar workspace: {e}"))?;
        fs::write(&path, raw).map_err(|e| format!("falha ao gravar workspace: {e}"))?;

        // Atualiza o índice
        self.update_index(workspace.to_list_item())?;
        Ok(())
    }

    /// Atualiza o índice de workspaces.
    fn update_index(&self, item: crate::models::WorkspaceListItem) -> Result<(), String> {
        let mut index = self.list_workspaces()?;
        if let Some(pos) = index.iter().position(|w| w.id == item.id) {
            index[pos] = item;
        } else {
            index.push(item);
        }
        // Ordena por last_opened_at decrescente (mais recente primeiro)
        index.sort_by(|a, b| b.last_opened_at.cmp(&a.last_opened_at));
        
        let raw = serde_json::to_string_pretty(&index)
            .map_err(|e| format!("falha ao serializar índice: {e}"))?;
        fs::write(index_path(&self.app_data_dir), raw).map_err(|e| format!("falha ao gravar índice: {e}"))
    }

    /// Remove um workspace.
    pub fn delete_workspace(&self, workspace_id: &str) -> Result<(), String> {
        self.ensure_initialized()?;
        let path = workspace_path(&self.app_data_dir, workspace_id);
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("falha ao remover workspace: {e}"))?;
        }
        // Remove do índice
        let mut index = self.list_workspaces()?;
        index.retain(|w| w.id != workspace_id);
        let raw = serde_json::to_string_pretty(&index)
            .map_err(|e| format!("falha ao serializar índice: {e}"))?;
        fs::write(index_path(&self.app_data_dir), raw).map_err(|e| format!("falha ao gravar índice: {e}"))?;

        // Remove arquivos auxiliares (eventos, memória e permissões) do workspace.
        self.delete_events(workspace_id);
        self.delete_memory_file(workspace_id);
        self.delete_permissions_file(workspace_id);

        Ok(())
    }

    /// Carrega o workspace mais recentemente aberto (para compatibilidade).
    pub fn load_last_workspace(&self) -> Result<Option<WorkspaceState>, String> {
        let workspaces = self.list_workspaces()?;
        if let Some(last) = workspaces.first() {
            self.load_workspace(&last.id)
        } else {
            Ok(None)
        }
    }

    /// Cria um backup timestamped do workspace atual.
    /// Retorna o caminho do arquivo de backup criado.
    pub fn backup_workspace(&self, workspace: &WorkspaceState) -> Result<String, String> {
        let backups_dir = self.app_data_dir.join("backups");
        fs::create_dir_all(&backups_dir)
            .map_err(|e| format!("falha ao criar diretório de backups: {e}"))?;

        let ts = now_millis();
        let filename = format!("{}-{}.json", workspace.metadata.id, ts);
        let path = backups_dir.join(&filename);

        let raw = serde_json::to_string_pretty(workspace)
            .map_err(|e| format!("falha ao serializar backup: {e}"))?;
        fs::write(&path, raw).map_err(|e| format!("falha ao gravar backup: {e}"))?;

        // Mantém apenas os últimos 10 backups por workspace
        self.prune_backups(&workspace.metadata.id, 10)?;

        Ok(path.to_string_lossy().to_string())
    }

    /// Remove backups antigos, mantendo apenas os `keep` mais recentes.
    fn prune_backups(&self, workspace_id: &str, keep: usize) -> Result<(), String> {
        let backups_dir = self.app_data_dir.join("backups");
        if !backups_dir.exists() {
            return Ok(());
        }

        let prefix = format!("{}-", workspace_id);
        let mut backups: Vec<_> = fs::read_dir(&backups_dir)
            .map_err(|e| format!("falha ao listar backups: {e}"))?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&prefix)
            })
            .collect();

        // Ordena por nome (timestamp no nome garante ordem cronológica)
        backups.sort_by_key(|e| e.file_name());

        // Remove os mais antigos
        let to_remove = backups.len().saturating_sub(keep);
        for entry in backups.into_iter().take(to_remove) {
            let _ = fs::remove_file(entry.path());
        }

        Ok(())
    }

    /// Lista os backups disponíveis para um workspace.
    pub fn list_backups(&self, workspace_id: &str) -> Result<Vec<String>, String> {
        let backups_dir = self.app_data_dir.join("backups");
        if !backups_dir.exists() {
            return Ok(Vec::new());
        }

        let prefix = format!("{}-", workspace_id);
        let mut backups: Vec<String> = fs::read_dir(&backups_dir)
            .map_err(|e| format!("falha ao listar backups: {e}"))?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&prefix)
            })
            .map(|entry| entry.path().to_string_lossy().to_string())
            .collect();

        // Ordena por nome (mais recente primeiro)
        backups.sort_by(|a, b| b.cmp(a));
        Ok(backups)
    }

    /// Migra o formato legado (v1, workspace único em `workspace.json`)
    /// para o novo formato multi-workspace. Roda apenas uma vez: se o
    /// índice já tiver workspaces, não faz nada.
    pub fn migrate_legacy(&self) -> Result<(), String> {
        self.ensure_initialized()?;

        // Já migrado (existe pelo menos um workspace no índice).
        let index = self.list_workspaces()?;
        if !index.is_empty() {
            return Ok(());
        }

        let legacy_path = self.app_data_dir.join("workspace.json");
        if !legacy_path.exists() {
            return Ok(());
        }

        let raw = fs::read_to_string(&legacy_path)
            .map_err(|e| format!("falha ao ler workspace legado: {e}"))?;
        let legacy: LegacyWorkspace = serde_json::from_str(&raw)
            .map_err(|e| format!("falha ao desserializar workspace legado: {e}"))?;

        let now = now_millis();
        let workspace = WorkspaceState {
            metadata: WorkspaceMetadata {
                id: format!("workspace-{}", now),
                name: if legacy.name.is_empty() {
                    "My Workspace".to_string()
                } else {
                    legacy.name
                },
                created_at: now,
                updated_at: legacy.updated_at.max(now),
                last_opened_at: now,
            },
            agents: legacy.agents,
            edges: legacy.edges,
            viewport: Viewport::default(),
            settings: WorkspaceSettings::default(),
            skills: Vec::new(),
            agent_skills: std::collections::HashMap::new(),
            mcp_servers: Vec::new(),
            agent_mcp: std::collections::HashMap::new(),
            skill_mcp: std::collections::HashMap::new(),
            version: if legacy.version.is_empty() {
                env!("CARGO_PKG_VERSION").to_string()
            } else {
                legacy.version
            },
        };

        self.save_workspace(&workspace)?;

        // Renomeia o arquivo antigo para evitar re-migração.
        let backup = self.app_data_dir.join("workspace.json.bak");
        fs::rename(&legacy_path, &backup)
            .map_err(|e| format!("falha ao arquivar workspace legado: {e}"))?;

        Ok(())
    }

    // ==================== EVENTS PERSISTENCE ====================

    fn events_dir(&self) -> PathBuf {
        self.app_data_dir.join("events")
    }

    fn events_path(&self, workspace_id: &str) -> PathBuf {
        self.events_dir().join(format!("{}.json", workspace_id))
    }

    /// Carrega os eventos persistidos de um workspace.
    pub fn load_events(&self, workspace_id: &str) -> Result<Vec<WorkflowEvent>, String> {
        let path = self.events_path(workspace_id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let raw = fs::read_to_string(&path).map_err(|e| format!("falha ao ler eventos: {e}"))?;
        serde_json::from_str(&raw).map_err(|e| format!("falha ao desserializar eventos: {e}"))
    }

    /// Carrega os eventos de TODOS os workspaces (hidratação no boot).
    pub fn load_all_events(&self) -> Result<Vec<WorkflowEvent>, String> {
        let dir = self.events_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut all = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|e| format!("falha ao listar eventos: {e}"))? {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(raw) = fs::read_to_string(&path) {
                if let Ok(events) = serde_json::from_str::<Vec<WorkflowEvent>>(&raw) {
                    all.extend(events);
                }
            }
        }
        Ok(all)
    }

    /// Anexa eventos a um workspace de forma SEGURA e LIMITADA:
    /// - grava em arquivo temporário e faz rename atômico (evita corrupção);
    /// - mantém apenas os N eventos mais recentes (não cresce indefinidamente).
    pub fn append_events(&self, workspace_id: &str, events: &[WorkflowEvent]) -> Result<(), String> {
        const MAX_EVENTS: usize = 500;

        let dir = self.events_dir();
        fs::create_dir_all(&dir).map_err(|e| format!("falha ao criar diretório de eventos: {e}"))?;

        let path = self.events_path(workspace_id);
        let mut existing: Vec<WorkflowEvent> = if path.exists() {
            let raw = fs::read_to_string(&path).unwrap_or_else(|_| "[]".to_string());
            serde_json::from_str(&raw).unwrap_or_default()
        } else {
            Vec::new()
        };

        existing.extend(events.iter().cloned());
        if existing.len() > MAX_EVENTS {
            let excess = existing.len() - MAX_EVENTS;
            existing.drain(0..excess);
        }

        let raw = serde_json::to_string_pretty(&existing)
            .map_err(|e| format!("falha ao serializar eventos: {e}"))?;
        let tmp = dir.join(format!("{}.tmp", workspace_id));
        fs::write(&tmp, raw).map_err(|e| format!("falha ao gravar eventos: {e}"))?;
        fs::rename(&tmp, &path).map_err(|e| format!("falha ao finalizar gravação de eventos: {e}"))
    }

    /// Remove os eventos persistidos de um workspace (limpeza ao deletar).
    pub fn delete_events(&self, workspace_id: &str) {
        let path = self.events_path(workspace_id);
        if path.exists() {
            let _ = fs::remove_file(&path);
        }
    }

    // ==================== MEMORY PERSISTENCE ====================

    fn memory_dir(&self) -> PathBuf {
        self.app_data_dir.join("memory")
    }

    fn memory_path(&self, workspace_id: &str) -> PathBuf {
        self.memory_dir().join(format!("{}.json", workspace_id))
    }

    /// Carrega as memórias persistidas de um workspace.
    pub fn load_memory(&self, workspace_id: &str) -> Result<Vec<MemoryEntry>, String> {
        let path = self.memory_path(workspace_id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let raw = fs::read_to_string(&path).map_err(|e| format!("falha ao ler memória: {e}"))?;
        serde_json::from_str(&raw).map_err(|e| format!("falha ao desserializar memória: {e}"))
    }

    /// Carrega as memórias de TODOS os workspaces (hidratação no boot).
    pub fn load_all_memory(&self) -> Result<Vec<MemoryEntry>, String> {
        let dir = self.memory_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut all = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|e| format!("falha ao listar memória: {e}"))? {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(raw) = fs::read_to_string(&path) {
                if let Ok(mems) = serde_json::from_str::<Vec<MemoryEntry>>(&raw) {
                    all.extend(mems);
                }
            }
        }
        Ok(all)
    }

    /// Salva as memórias de um workspace de forma SEGURA e LIMITADA:
    /// grava em arquivo temporário e faz rename atômico (evita corrupção) e
    /// mantém apenas as N entradas mais recentes (não cresce indefinidamente).
    pub fn save_memory(&self, workspace_id: &str, entries: &[MemoryEntry]) -> Result<(), String> {
        const MAX_MEMORY: usize = 500;

        let dir = self.memory_dir();
        fs::create_dir_all(&dir).map_err(|e| format!("falha ao criar diretório de memória: {e}"))?;

        // Entradas ordenadas cronologicamente: mantém as mais recentes.
        let mut existing: Vec<MemoryEntry> = entries.to_vec();
        existing.sort_by_key(|m| m.timestamp);
        if existing.len() > MAX_MEMORY {
            let excess = existing.len() - MAX_MEMORY;
            existing.drain(0..excess);
        }

        let raw = serde_json::to_string_pretty(&existing)
            .map_err(|e| format!("falha ao serializar memória: {e}"))?;
        let path = self.memory_path(workspace_id);
        let tmp = dir.join(format!("{}.tmp", workspace_id));
        fs::write(&tmp, raw).map_err(|e| format!("falha ao gravar memória: {e}"))?;
        fs::rename(&tmp, &path)
            .map_err(|e| format!("falha ao finalizar gravação de memória: {e}"))
    }

    /// Remove as memórias persistidas de um workspace (limpeza ao deletar).
    pub fn delete_memory_file(&self, workspace_id: &str) {
        let path = self.memory_path(workspace_id);
        if path.exists() {
            let _ = fs::remove_file(&path);
        }
    }

    // ==================== PERMISSIONS PERSISTENCE ====================

    fn permissions_dir(&self) -> PathBuf {
        self.app_data_dir.join("permissions")
    }

    fn permissions_path(&self, workspace_id: &str) -> PathBuf {
        self.permissions_dir().join(format!("{}.json", workspace_id))
    }

    /// Carrega as permissões persistidas de TODOS os workspaces.
    pub fn load_all_permissions(&self) -> Result<Vec<PermissionGrant>, String> {
        let dir = self.permissions_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut all = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|e| format!("falha ao listar permissões: {e}"))? {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(raw) = fs::read_to_string(&path) {
                if let Ok(grants) = serde_json::from_str::<Vec<PermissionGrant>>(&raw) {
                    all.extend(grants);
                }
            }
        }
        Ok(all)
    }

    /// Salva as permissões de um workspace com rename atômico.
    pub fn save_permissions(&self, workspace_id: &str, grants: &[PermissionGrant]) -> Result<(), String> {
        let dir = self.permissions_dir();
        fs::create_dir_all(&dir).map_err(|e| format!("falha ao criar diretório de permissões: {e}"))?;
        let raw = serde_json::to_string_pretty(grants)
            .map_err(|e| format!("falha ao serializar permissões: {e}"))?;
        let path = self.permissions_path(workspace_id);
        let tmp = dir.join(format!("{}.tmp", workspace_id));
        fs::write(&tmp, raw).map_err(|e| format!("falha ao gravar permissões: {e}"))?;
        fs::rename(&tmp, &path)
            .map_err(|e| format!("falha ao finalizar gravação de permissões: {e}"))
    }

    /// Remove as permissões persistidas de um workspace.
    pub fn delete_permissions_file(&self, workspace_id: &str) {
        let path = self.permissions_path(workspace_id);
        if path.exists() {
            let _ = fs::remove_file(&path);
        }
    }

    // ==================== PROJECT PERSISTENCE ====================

    fn projects_path(&self) -> PathBuf {
        self.app_data_dir.join("projects.json")
    }

    /// Carrega todos os projetos. Arquivo ausente => lista vazia.
    pub fn list_projects(&self) -> Result<Vec<Project>, String> {
        let path = self.projects_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let raw = fs::read_to_string(&path).map_err(|e| format!("falha ao ler projetos: {e}"))?;
        serde_json::from_str(&raw).map_err(|e| format!("falha ao desserializar projetos: {e}"))
    }

    /// Carrega um projeto por id (Some/None).
    pub fn load_project(&self, id: &str) -> Result<Option<Project>, String> {
        Ok(self.list_projects()?.into_iter().find(|p| p.id == id))
    }

    /// Grava a lista de projetos de forma atômica (tmp -> rename).
    fn write_projects(&self, projects: &[Project]) -> Result<(), String> {
        let path = self.projects_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("falha ao criar diretório de projetos: {e}"))?;
        }
        let raw = serde_json::to_string_pretty(projects)
            .map_err(|e| format!("falha ao serializar projetos: {e}"))?;
        let tmp = self.app_data_dir.join("projects.json.tmp");
        fs::write(&tmp, raw).map_err(|e| format!("falha ao gravar projetos: {e}"))?;
        fs::rename(&tmp, &path)
            .map_err(|e| format!("falha ao finalizar gravação de projetos: {e}"))
    }

    /// Cria ou atualiza um projeto (upsert por id).
    pub fn save_project(&self, project: &Project) -> Result<(), String> {
        let mut projects = self.list_projects()?;
        if let Some(pos) = projects.iter().position(|p| p.id == project.id) {
            projects[pos] = project.clone();
        } else {
            projects.push(project.clone());
        }
        self.write_projects(&projects)
    }

    /// Remove um projeto. Retorna Err se não existir.
    pub fn delete_project(&self, id: &str) -> Result<(), String> {
        let mut projects = self.list_projects()?;
        let before = projects.len();
        projects.retain(|p| p.id != id);
        if projects.len() == before {
            return Err("projeto não encontrado".to_string());
        }
        self.write_projects(&projects)
    }

    // ==================== WORKFLOW PERSISTENCE ====================

    fn workflows_dir(&self) -> PathBuf {
        self.app_data_dir.join("workflows")
    }

    fn workflows_index_path(&self) -> PathBuf {
        self.workflows_dir().join("index.json")
    }

    fn workflow_path(&self, workflow_id: &str) -> PathBuf {
        self.workflows_dir().join(format!("{}.json", workflow_id))
    }

    fn executions_dir(&self) -> PathBuf {
        self.app_data_dir.join("executions")
    }

    fn executions_index_path(&self) -> PathBuf {
        self.executions_dir().join("index.json")
    }

    fn execution_path(&self, execution_id: &str) -> PathBuf {
        self.executions_dir().join(format!("{}.json", execution_id))
    }

    fn ensure_workflows_initialized(&self) -> Result<(), String> {
        let dir = self.workflows_dir();
        fs::create_dir_all(&dir).map_err(|e| format!("falha ao criar diretório de workflows: {e}"))?;
        let index_file = self.workflows_index_path();
        if !index_file.exists() {
            let initial: Vec<WorkflowDefinition> = Vec::new();
            let raw = serde_json::to_string_pretty(&initial)
                .map_err(|e| format!("falha ao serializar índice de workflows: {e}"))?;
            fs::write(&index_file, raw).map_err(|e| format!("falha ao gravar índice de workflows: {e}"))?;
        }
        Ok(())
    }

    fn ensure_executions_initialized(&self) -> Result<(), String> {
        let dir = self.executions_dir();
        fs::create_dir_all(&dir).map_err(|e| format!("falha ao criar diretório de execuções: {e}"))?;
        let index_file = self.executions_index_path();
        if !index_file.exists() {
            let initial: Vec<WorkflowExecution> = Vec::new();
            let raw = serde_json::to_string_pretty(&initial)
                .map_err(|e| format!("falha ao serializar índice de execuções: {e}"))?;
            fs::write(&index_file, raw).map_err(|e| format!("falha ao gravar índice de execuções: {e}"))?;
        }
        Ok(())
    }

    pub fn save_workflow(&self, workflow: &WorkflowDefinition) -> Result<(), String> {
        self.ensure_workflows_initialized()?;
        let path = self.workflow_path(&workflow.id);
        let raw = serde_json::to_string_pretty(workflow)
            .map_err(|e| format!("falha ao serializar workflow: {e}"))?;
        fs::write(&path, raw).map_err(|e| format!("falha ao gravar workflow: {e}"))?;

        // Atualiza índice
        let mut index: Vec<WorkflowDefinition> = self.list_workflows()?;
        if let Some(pos) = index.iter().position(|w| w.id == workflow.id) {
            index[pos] = workflow.clone();
        } else {
            index.push(workflow.clone());
        }
        index.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        let raw = serde_json::to_string_pretty(&index)
            .map_err(|e| format!("falha ao serializar índice de workflows: {e}"))?;
        fs::write(self.workflows_index_path(), raw).map_err(|e| format!("falha ao gravar índice de workflows: {e}"))
    }

    pub fn load_workflow(&self, workflow_id: &str) -> Result<Option<WorkflowDefinition>, String> {
        self.ensure_workflows_initialized()?;
        let path = self.workflow_path(workflow_id);
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path).map_err(|e| format!("falha ao ler workflow: {e}"))?;
        let workflow = serde_json::from_str(&raw).map_err(|e| format!("falha ao desserializar workflow: {e}"))?;
        Ok(Some(workflow))
    }

    pub fn list_workflows(&self) -> Result<Vec<WorkflowDefinition>, String> {
        self.ensure_workflows_initialized()?;
        let raw = fs::read_to_string(self.workflows_index_path())
            .map_err(|e| format!("falha ao ler índice de workflows: {e}"))?;
        serde_json::from_str(&raw).map_err(|e| format!("falha ao desserializar índice de workflows: {e}"))
    }

    pub fn delete_workflow(&self, workflow_id: &str) -> Result<(), String> {
        self.ensure_workflows_initialized()?;
        let path = self.workflow_path(workflow_id);
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("falha ao remover workflow: {e}"))?;
        }
        let mut index = self.list_workflows()?;
        index.retain(|w| w.id != workflow_id);
        let raw = serde_json::to_string_pretty(&index)
            .map_err(|e| format!("falha ao serializar índice de workflows: {e}"))?;
        fs::write(self.workflows_index_path(), raw).map_err(|e| format!("falha ao gravar índice de workflows: {e}"))
    }

    // ==================== EXECUTION PERSISTENCE ====================

    pub fn save_execution(&self, execution: &WorkflowExecution) -> Result<(), String> {
        self.ensure_executions_initialized()?;
        let path = self.execution_path(&execution.id);
        let raw = serde_json::to_string_pretty(execution)
            .map_err(|e| format!("falha ao serializar execução: {e}"))?;
        fs::write(&path, raw).map_err(|e| format!("falha ao gravar execução: {e}"))?;

        // Atualiza índice
        let mut index: Vec<WorkflowExecution> = self.list_executions()?;
        if let Some(pos) = index.iter().position(|e| e.id == execution.id) {
            index[pos] = execution.clone();
        } else {
            index.push(execution.clone());
        }
        index.sort_by(|a, b| b.started_at.unwrap_or(0).cmp(&a.started_at.unwrap_or(0)));
        let raw = serde_json::to_string_pretty(&index)
            .map_err(|e| format!("falha ao serializar índice de execuções: {e}"))?;
        fs::write(self.executions_index_path(), raw).map_err(|e| format!("falha ao gravar índice de execuções: {e}"))
    }

    pub fn load_execution(&self, execution_id: &str) -> Result<Option<WorkflowExecution>, String> {
        self.ensure_executions_initialized()?;
        let path = self.execution_path(execution_id);
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path).map_err(|e| format!("falha ao ler execução: {e}"))?;
        let execution = serde_json::from_str(&raw).map_err(|e| format!("falha ao desserializar execução: {e}"))?;
        Ok(Some(execution))
    }

    pub fn list_executions(&self) -> Result<Vec<WorkflowExecution>, String> {
        self.ensure_executions_initialized()?;
        let raw = fs::read_to_string(self.executions_index_path())
            .map_err(|e| format!("falha ao ler índice de execuções: {e}"))?;
        serde_json::from_str(&raw).map_err(|e| format!("falha ao desserializar índice de execuções: {e}"))
    }

    pub fn delete_execution(&self, execution_id: &str) -> Result<(), String> {
        self.ensure_executions_initialized()?;
        let path = self.execution_path(execution_id);
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("falha ao remover execução: {e}"))?;
        }
        let mut index = self.list_executions()?;
        index.retain(|e| e.id != execution_id);
        let raw = serde_json::to_string_pretty(&index)
            .map_err(|e| format!("falha ao serializar índice de execuções: {e}"))?;
        fs::write(self.executions_index_path(), raw).map_err(|e| format!("falha ao gravar índice de execuções: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::WorkspaceState;

    fn temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "maestri-test-{}-{}-{:?}",
            now_millis(),
            SEQ.fetch_add(1, Ordering::SeqCst),
            std::thread::current().id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_and_load_workspace_roundtrip() {
        let dir = temp_dir();
        let p = Persistence::new(dir.clone());

        let mut ws = WorkspaceState::new("Test WS".to_string());
        ws.agents.push(Agent {
            id: "a1".to_string(),
            name: "Agent 1".to_string(),
            role: crate::models::Role::Builder,
            runtime: crate::models::Runtime::Custom,
            kind: crate::models::AgentKind::Cli,
            model: "m".to_string(),
            command: "echo".to_string(),
            args: vec!["hi".to_string()],
            working_dir: "".to_string(),
            auto_start: false,
            status: crate::models::Status::Idle,
            x: 0.1,
            y: 0.2,
            width: Some(520.0),
            height: Some(360.0),
            collapsed: false,
            locked: false,
            accent: None,
        });

        p.save_workspace(&ws).unwrap();
        let loaded = p.load_workspace(&ws.metadata.id).unwrap().unwrap();
        assert_eq!(loaded.metadata.name, "Test WS");
        assert_eq!(loaded.agents.len(), 1);
        assert_eq!(loaded.agents[0].id, "a1");
        assert_eq!(loaded.agents[0].width, Some(520.0));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_workspaces_updates_index() {
        let dir = temp_dir();
        let p = Persistence::new(dir.clone());

        let ws1 = WorkspaceState::new("WS1".to_string());
        std::thread::sleep(std::time::Duration::from_millis(2));
        let ws2 = WorkspaceState::new("WS2".to_string());
        p.save_workspace(&ws1).unwrap();
        p.save_workspace(&ws2).unwrap();

        let list = p.list_workspaces().unwrap();
        assert_eq!(list.len(), 2);

        p.delete_workspace(&ws1.metadata.id).unwrap();
        let list = p.list_workspaces().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "WS2");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_creates_file_and_prunes() {
        let dir = temp_dir();
        let p = Persistence::new(dir.clone());
        let ws = WorkspaceState::new("Backup WS".to_string());

        // Cria 12 backups — deve manter apenas 10
        for _ in 0..12 {
            p.backup_workspace(&ws).unwrap();
            // Garante timestamps distintos
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        let backups = p.list_backups(&ws.metadata.id).unwrap();
        assert_eq!(backups.len(), 10);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_nonexistent_returns_none() {
        let dir = temp_dir();
        let p = Persistence::new(dir.clone());
        assert!(p.load_workspace("nonexistent").unwrap().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    fn proj(name: &str) -> Project {
        Project::new(name.to_string(), now_millis())
    }

    #[test]
    fn project_save_list_and_reload() {
        let dir = temp_dir();
        let p = Persistence::new(dir.clone());
        assert!(p.list_projects().unwrap().is_empty());

        let a = proj("P1");
        p.save_project(&a).unwrap();
        p.save_project(&proj("P2")).unwrap();

        let list = p.list_projects().unwrap();
        assert_eq!(list.len(), 2);

        let loaded = p.load_project(&a.id).unwrap().unwrap();
        assert_eq!(loaded.name, "P1");

        // Simula reinício: nova instância no mesmo diretório.
        let p2 = Persistence::new(dir.clone());
        assert_eq!(p2.list_projects().unwrap().len(), 2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn project_upsert_over_writes_by_id() {
        let dir = temp_dir();
        let p = Persistence::new(dir.clone());

        let mut a = proj("P1");
        p.save_project(&a).unwrap();
        a.name = "P1 renomeado".to_string();
        p.save_project(&a).unwrap();

        let list = p.list_projects().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "P1 renomeado");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn project_delete_removes_only_target() {
        let dir = temp_dir();
        let p = Persistence::new(dir.clone());

        let a = proj("P1");
        let b = proj("P2");
        p.save_project(&a).unwrap();
        p.save_project(&b).unwrap();

        p.delete_project(&a.id).unwrap();
        let list = p.list_projects().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, b.id);

        assert!(p.delete_project(&a.id).is_err()); // já removido

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn project_membership_persists() {
        let dir = temp_dir();
        let p = Persistence::new(dir.clone());

        let mut a = proj("P1");
        a.add_workspace("ws-1");
        a.add_workspace("ws-2");
        p.save_project(&a).unwrap();

        let loaded = p.load_project(&a.id).unwrap().unwrap();
        assert_eq!(loaded.workspace_ids, vec!["ws-1", "ws-2"]);

        let _ = fs::remove_dir_all(&dir);
    }

    fn evt(ws: &str, ev: crate::events::WorkflowEventType) -> WorkflowEvent {
        WorkflowEvent {
            id: format!("id-{}", now_millis()),
            workspace_id: ws.to_string(),
            workflow_id: "w1".to_string(),
            event: ev,
            agent_id: None,
            step: None,
            data: None,
            timestamp: now_millis(),
        }
    }

    #[test]
    fn append_and_load_events_roundtrip() {
        let dir = temp_dir();
        let p = Persistence::new(dir.clone());
        let events = vec![
            evt("ws-A", crate::events::WorkflowEventType::WorkflowStarted),
            evt("ws-A", crate::events::WorkflowEventType::WorkflowCompleted),
        ];
        p.append_events("ws-A", &events).unwrap();

        let loaded = p.load_events("ws-A").unwrap();
        assert_eq!(loaded.len(), 2);
        assert!(loaded.iter().all(|e| e.workspace_id == "ws-A"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn events_isolated_by_workspace() {
        let dir = temp_dir();
        let p = Persistence::new(dir.clone());
        p.append_events("ws-A", &[evt("ws-A", crate::events::WorkflowEventType::WorkflowStarted)]).unwrap();
        p.append_events("ws-B", &[evt("ws-B", crate::events::WorkflowEventType::WorkflowStarted)]).unwrap();

        let a = p.load_events("ws-A").unwrap();
        let b = p.load_events("ws-B").unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert!(a.iter().all(|e| e.workspace_id == "ws-A"));
        assert!(b.iter().all(|e| e.workspace_id == "ws-B"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn events_are_bounded() {
        let dir = temp_dir();
        let p = Persistence::new(dir.clone());
        let mut many = Vec::new();
        for _ in 0..600 {
            many.push(evt("ws-A", crate::events::WorkflowEventType::WorkflowStarted));
        }
        p.append_events("ws-A", &many).unwrap();

        let loaded = p.load_events("ws-A").unwrap();
        assert_eq!(loaded.len(), 500); // MAX_EVENTS
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn events_survive_reload_new_instance() {
        let dir = temp_dir();
        let p = Persistence::new(dir.clone());
        p.append_events("ws-A", &[evt("ws-A", crate::events::WorkflowEventType::StepFailed)]).unwrap();

        // Simula reinício: nova instância no mesmo diretório.
        let p2 = Persistence::new(dir.clone());
        let all = p2.load_all_events().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].event, crate::events::WorkflowEventType::StepFailed);
        let _ = fs::remove_dir_all(&dir);
    }

    fn mem(ws: &str, id: &str, content: &str) -> MemoryEntry {
        MemoryEntry {
            id: id.to_string(),
            workspace_id: ws.to_string(),
            agent_id: None,
            category: "fato".to_string(),
            content: content.to_string(),
            metadata: std::collections::HashMap::new(),
            timestamp: now_millis(),
        }
    }

    #[test]
    fn save_and_load_memory_roundtrip() {
        let dir = temp_dir();
        let p = Persistence::new(dir.clone());
        p.save_memory("ws-A", &[mem("ws-A", "m1", "fato 1"), mem("ws-A", "m2", "fato 2")]).unwrap();
        let loaded = p.load_memory("ws-A").unwrap();
        assert_eq!(loaded.len(), 2);
        assert!(loaded.iter().all(|m| m.workspace_id == "ws-A"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn memory_isolated_by_workspace() {
        let dir = temp_dir();
        let p = Persistence::new(dir.clone());
        p.save_memory("ws-A", &[mem("ws-A", "m1", "A")]).unwrap();
        p.save_memory("ws-B", &[mem("ws-B", "m2", "B")]).unwrap();
        assert_eq!(p.load_memory("ws-A").unwrap().len(), 1);
        assert_eq!(p.load_memory("ws-B").unwrap().len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn memory_is_bounded() {
        let dir = temp_dir();
        let p = Persistence::new(dir.clone());
        let mut many: Vec<MemoryEntry> = Vec::new();
        for i in 0..600 {
            many.push(mem("ws-A", &format!("m{}", i), "x"));
        }
        p.save_memory("ws-A", &many).unwrap();
        assert!(p.load_memory("ws-A").unwrap().len() <= 500);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn memory_survives_reload_new_instance() {
        let dir = temp_dir();
        let p = Persistence::new(dir.clone());
        p.save_memory("ws-A", &[mem("ws-A", "m1", "persiste")]).unwrap();
        let p2 = Persistence::new(dir.clone());
        let all = p2.load_all_memory().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].content, "persiste");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn permissions_survive_reload_new_instance() {
        let dir = temp_dir();
        let p = Persistence::new(dir.clone());
        let grants = vec![PermissionGrant {
            workspace_id: "ws-A".to_string(),
            agent_id: "a1".to_string(),
            permission: crate::permissions::Permission::BrowserControl,
        }];
        p.save_permissions("ws-A", &grants).unwrap();

        let p2 = Persistence::new(dir.clone());
        let all = p2.load_all_permissions().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].agent_id, "a1");
        assert_eq!(all[0].permission, crate::permissions::Permission::BrowserControl);
        let _ = fs::remove_dir_all(&dir);
    }
}