//! Memory (Fase 11) — primeira camada de memória para agentes.
//!
//! Modelo simples: `MemoryEntry` persistida e isolada por workspace, com
//! entradas consultáveis por agente ou compartilhadas (workspace). Sem
//! embeddings/vector DB/RAG.
//!
//! Escopo de acesso:
//! - `memory_by_agent(id)` => compartilhadas (agent_id = None) + privadas do
//!   próprio agente. Um agente NÃO consulta memória privada de outro agente.
//! - `list_memory` => todas as entradas do workspace (view do operador).
//! - `memory_shared` => somente entradas compartilhadas do workspace.
//!
//! Fora de escopo: Supabase/cloud, RAG, embeddings, Marketplace, Web/App.

use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::State;

/// Entrada de memória.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub workspace_id: String,
    /// None => memória compartilhada do workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Tipo/categoria (ex.: "fato", "preferência", "contexto").
    pub category: String,
    pub content: String,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
    pub timestamp: u64,
}

/// Payload de criação de memória.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateMemoryInput {
    /// None => compartilhada.
    #[serde(default)]
    pub agent_id: Option<String>,
    pub category: String,
    pub content: String,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

fn current_workspace_id(state: &AppState) -> String {
    state
        .workspace
        .lock()
        .expect("workspace mutex poisoned")
        .metadata
        .id
        .clone()
}

/// Lista todas as memórias do workspace atual (view do operador).
#[tauri::command]
pub fn list_memory(state: State<'_, AppState>) -> Result<Vec<MemoryEntry>, String> {
    let ws = current_workspace_id(&state);
    state.persistence.load_memory(&ws)
}

/// Memórias acessíveis por um agente: compartilhadas + as privadas dele.
#[tauri::command]
pub fn memory_by_agent(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<Vec<MemoryEntry>, String> {
    let ws = current_workspace_id(&state);
    let all = state.persistence.load_memory(&ws)?;
    Ok(all
        .into_iter()
        .filter(|m| m.agent_id.as_deref() == Some(&agent_id) || m.agent_id.is_none())
        .collect())
}

/// Memórias compartilhadas do workspace.
#[tauri::command]
pub fn memory_shared(state: State<'_, AppState>) -> Result<Vec<MemoryEntry>, String> {
    let ws = current_workspace_id(&state);
    let all = state.persistence.load_memory(&ws)?;
    Ok(all.into_iter().filter(|m| m.agent_id.is_none()).collect())
}

/// Cria e persiste uma entrada de memória.
#[tauri::command]
pub fn create_memory(
    state: State<'_, AppState>,
    input: CreateMemoryInput,
) -> Result<MemoryEntry, String> {
    if input.content.trim().is_empty() {
        return Err("conteúdo da memória é obrigatório".to_string());
    }
    if input.category.trim().is_empty() {
        return Err("categoria da memória é obrigatória".to_string());
    }

    let ws = current_workspace_id(&state);

    // Valida agente (se informado) dentro do workspace.
    if let Some(agent_id) = &input.agent_id {
        let guard = state.workspace.lock().expect("workspace mutex poisoned");
        if !guard.agents.iter().any(|a| &a.id == agent_id) {
            return Err("agente não encontrado no workspace".to_string());
        }
    }

    let entry = MemoryEntry {
        id: crate::workflow::gen_id("mem"),
        workspace_id: ws.clone(),
        agent_id: input.agent_id.clone(),
        category: input.category.trim().to_string(),
        content: input.content.trim().to_string(),
        metadata: input.metadata.clone(),
        timestamp: crate::workflow::now_millis(),
    };

    // Anexa e persiste (a persistência aplica o limite por workspace).
    let mut entries = state.persistence.load_memory(&ws)?;
    entries.push(entry.clone());
    state.persistence.save_memory(&ws, &entries)?;

    Ok(entry)
}

/// Remove uma entrada de memória pelo id (escopado ao workspace atual).
#[tauri::command]
pub fn remove_memory(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let ws = current_workspace_id(&state);
    let mut entries = state.persistence.load_memory(&ws)?;
    let before = entries.len();
    entries.retain(|m| m.id != id);
    if entries.len() == before {
        return Err("memória não encontrada".to_string());
    }
    state.persistence.save_memory(&ws, &entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, ws: &str, agent_id: Option<&str>, content: &str) -> MemoryEntry {
        MemoryEntry {
            id: id.to_string(),
            workspace_id: ws.to_string(),
            agent_id: agent_id.map(|s| s.to_string()),
            category: "fato".to_string(),
            content: content.to_string(),
            metadata: HashMap::new(),
            timestamp: 1,
        }
    }

    #[test]
    fn memory_entry_serializes_roundtrip() {
        let m = entry("m1", "ws-A", Some("a1"), "lembra isto");
        let json = serde_json::to_string(&m).unwrap();
        let back: MemoryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);

        // agent_id None => omitido no JSON.
        let shared = entry("m2", "ws-A", None, "shared");
        let sj = serde_json::to_string(&shared).unwrap();
        assert!(!sj.contains("agent_id"));
        assert_eq!(serde_json::from_str::<MemoryEntry>(&sj).unwrap(), shared);
    }

    #[test]
    fn memory_transport_scoping_by_agent() {
        let entries = vec![
            entry("m1", "ws-A", Some("a1"), "privado A"),
            entry("m2", "ws-A", Some("a2"), "privado B"),
            entry("m3", "ws-A", None, "shared"),
        ];

        // Agent A vê: privado dele + shared. NÃO vê privado de B.
        let a: Vec<&MemoryEntry> = entries
            .iter()
            .filter(|m| m.agent_id.as_deref() == Some("a1") || m.agent_id.is_none())
            .collect();
        assert_eq!(a.len(), 2);
        assert!(a.iter().all(|m| m.agent_id.as_deref() != Some("a2")));

        // Shared (workspace) => só agent_id None.
        let shared: Vec<&MemoryEntry> = entries.iter().filter(|m| m.agent_id.is_none()).collect();
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].id, "m3");
    }

    #[test]
    fn memory_isolated_between_workspaces_by_workspace_id() {
        let e1 = entry("m1", "ws-A", None, "A");
        let e2 = entry("m2", "ws-B", None, "B");
        assert_eq!(e1.workspace_id, "ws-A");
        assert_eq!(e2.workspace_id, "ws-B");
        assert_ne!(e1.workspace_id, e2.workspace_id);
    }
}