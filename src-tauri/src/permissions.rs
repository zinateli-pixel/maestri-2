//! Fase 17 — Permissões / Segurança.
//!
//! Camada de autorização que antecede as capacidades de controle do computador
//! (Fase 16): um agente só pode acionar ações sensíveis (navegador, aplicativo,
//! rede, shell, arquivos) se houver uma concessão explícita no workspace atual.
//!
//! Modelo: DENY por padrão. Toda ação perigosa exige `Permission` concedida
//! para `(workspace_id, agent_id)`. O registro é persistido por workspace e
//! isolado: uma concessão em um workspace nunca vaza para outro.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use crate::state::AppState;
use tauri::State;

/// Ação sensível que um agente pode tentar executar fora do seu processo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Controlar o navegador (clicar, digitar, navegar) — Fase 16.
    BrowserControl,
    /// Controlar aplicativos do sistema (launch/focus/tap) — Fase 16.
    AppControl,
    /// Executar comandos de shell arbitrários.
    Shell,
    /// Acesso à rede.
    Network,
    /// Ler/escrever arquivos fora do workspace do agente.
    Filesystem,
}

impl Permission {
    pub fn as_str(&self) -> &'static str {
        match self {
            Permission::BrowserControl => "browser_control",
            Permission::AppControl => "app_control",
            Permission::Shell => "shell",
            Permission::Network => "network",
            Permission::Filesystem => "filesystem",
        }
    }

    /// Conjunto de todas as permissões conhecidas (para listagem/validação).
    pub fn all() -> [Permission; 5] {
        [
            Permission::BrowserControl,
            Permission::AppControl,
            Permission::Shell,
            Permission::Network,
            Permission::Filesystem,
        ]
    }
}

/// Uma concessão persistida: agente `agent_id` pode executar `permission`
/// no `workspace_id` indicado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PermissionGrant {
    pub workspace_id: String,
    pub agent_id: String,
    pub permission: Permission,
}

/// Registro de permissões em memória, escopado por workspace + agente.
#[derive(Default)]
pub struct PermissionRegistry {
    /// (workspace_id, agent_id) -> conjunto de permissões concedidas.
    grants: Mutex<HashMap<(String, String), HashSet<Permission>>>,
}

impl PermissionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn snapshot(&self) -> HashMap<(String, String), HashSet<Permission>> {
        self.grants
            .lock()
            .expect("permissions mutex poisoned")
            .clone()
    }

    /// Concede uma permissão a um agente num workspace.
    pub fn grant(&self, workspace_id: &str, agent_id: &str, permission: Permission) {
        self.grants
            .lock()
            .expect("permissions mutex poisoned")
            .entry((workspace_id.to_string(), agent_id.to_string()))
            .or_default()
            .insert(permission);
    }

    /// Revoga uma permissão de um agente num workspace.
    pub fn revoke(&self, workspace_id: &str, agent_id: &str, permission: Permission) {
        if let Some(set) = self
            .grants
            .lock()
            .expect("permissions mutex poisoned")
            .get_mut(&(workspace_id.to_string(), agent_id.to_string()))
        {
            set.remove(&permission);
        }
    }

    /// Verifica se a permissão foi explicitamente concedida (deny por padrão).
    pub fn is_granted(&self, workspace_id: &str, agent_id: &str, permission: Permission) -> bool {
        self.snapshot()
            .get(&(workspace_id.to_string(), agent_id.to_string()))
            .map(|set| set.contains(&permission))
            .unwrap_or(false)
    }

    /// Permissões concedidas a um agente num workspace.
    pub fn permissions_of(&self, workspace_id: &str, agent_id: &str) -> Vec<Permission> {
        let mut perms: Vec<Permission> = self
            .snapshot()
            .get(&(workspace_id.to_string(), agent_id.to_string()))
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default();
        perms.sort_by_key(|p| p.as_str());
        perms
    }

    /// Exporta todas as concessões (para persistência). Isolamento por
    /// workspace garantido pela chave composta.
    pub fn all_grants(&self) -> Vec<PermissionGrant> {
        let mut out = Vec::new();
        for ((workspace_id, agent_id), set) in self.snapshot() {
            for permission in set {
                out.push(PermissionGrant {
                    workspace_id: workspace_id.clone(),
                    agent_id: agent_id.clone(),
                    permission,
                });
            }
        }
        out.sort_by(|a, b| {
            (&a.workspace_id, &a.agent_id, a.permission.as_str())
                .cmp(&(&b.workspace_id, &b.agent_id, b.permission.as_str()))
        });
        out
    }

    /// Hidrata o registro a partir de concessões persistidas.
    pub fn load(&self, grants: &[PermissionGrant]) {
        let mut guard = self.grants.lock().expect("permissions mutex poisoned");
        for g in grants {
            guard
                .entry((g.workspace_id.clone(), g.agent_id.clone()))
                .or_default()
                .insert(g.permission);
        }
    }
}

// ==================== TAURI COMMANDS ====================

fn current_workspace_id(state: &AppState) -> String {
    state
        .workspace
        .lock()
        .expect("workspace mutex poisoned")
        .metadata
        .id
        .clone()
}

/// Persiste as concessões do workspace atual (filtra por isolamento).
fn persist_workspace(state: &AppState, workspace_id: &str) -> Result<(), String> {
    let grants: Vec<PermissionGrant> = state
        .permissions
        .all_grants()
        .into_iter()
        .filter(|g| g.workspace_id == workspace_id)
        .collect();
    state.persistence.save_permissions(workspace_id, &grants)
}

/// Lista as permissões concedidas no workspace atual.
#[tauri::command]
pub fn list_permissions(state: State<'_, AppState>) -> Vec<PermissionGrant> {
    let ws = current_workspace_id(&state);
    state
        .permissions
        .all_grants()
        .into_iter()
        .filter(|g| g.workspace_id == ws)
        .collect()
}

/// Permissões concedidas a um agente no workspace atual.
#[tauri::command]
pub fn permissions_of_agent(state: State<'_, AppState>, agent_id: String) -> Vec<Permission> {
    let ws = current_workspace_id(&state);
    state.permissions.permissions_of(&ws, &agent_id)
}

/// Concede uma permissão a um agente no workspace atual (deny por padrão).
#[tauri::command]
pub fn grant_permission(
    state: State<'_, AppState>,
    agent_id: String,
    permission: Permission,
) -> Result<(), String> {
    let ws = current_workspace_id(&state);
    {
        let guard = state.workspace.lock().expect("workspace mutex poisoned");
        if !guard.agents.iter().any(|a| a.id == agent_id) {
            return Err("agente não encontrado no workspace".to_string());
        }
    }
    state.permissions.grant(&ws, &agent_id, permission);
    persist_workspace(&state, &ws)
}

/// Revoga uma permissão de um agente no workspace atual.
#[tauri::command]
pub fn revoke_permission(
    state: State<'_, AppState>,
    agent_id: String,
    permission: Permission,
) -> Result<(), String> {
    let ws = current_workspace_id(&state);
    state.permissions.revoke(&ws, &agent_id, permission);
    persist_workspace(&state, &ws)
}

/// Verifica se um agente tem uma permissão no workspace atual.
#[tauri::command]
pub fn check_permission(
    state: State<'_, AppState>,
    agent_id: String,
    permission: Permission,
) -> Result<bool, String> {
    let ws = current_workspace_id(&state);
    Ok(state.permissions.is_granted(&ws, &agent_id, permission))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_by_default() {
        let reg = PermissionRegistry::new();
        assert!(!reg.is_granted("ws-a", "a1", Permission::BrowserControl));
        assert!(reg.permissions_of("ws-a", "a1").is_empty());
    }

    #[test]
    fn grant_revoke_check_roundtrip() {
        let reg = PermissionRegistry::new();
        reg.grant("ws-a", "a1", Permission::BrowserControl);
        assert!(reg.is_granted("ws-a", "a1", Permission::BrowserControl));
        assert!(!reg.is_granted("ws-a", "a1", Permission::AppControl));

        reg.revoke("ws-a", "a1", Permission::BrowserControl);
        assert!(!reg.is_granted("ws-a", "a1", Permission::BrowserControl));
    }

    #[test]
    fn isolation_between_workspaces() {
        let reg = PermissionRegistry::new();
        reg.grant("ws-a", "a1", Permission::Shell);
        assert!(reg.is_granted("ws-a", "a1", Permission::Shell));
        // Mesmo agente id num workspace diferente: não herda a concessão.
        assert!(!reg.is_granted("ws-b", "a1", Permission::Shell));
    }

    #[test]
    fn grants_survive_reload() {
        let reg = PermissionRegistry::new();
        reg.grant("ws-a", "a1", Permission::Network);
        let grants = reg.all_grants();

        let reg2 = PermissionRegistry::new();
        reg2.load(&grants);
        assert!(reg2.is_granted("ws-a", "a1", Permission::Network));
    }

    #[test]
    fn permission_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&Permission::BrowserControl).unwrap(),
            "\"browser_control\""
        );
        assert_eq!(
            serde_json::to_string(&Permission::AppControl).unwrap(),
            "\"app_control\""
        );
    }
}