//! Fase 18 — History / Replay.
//!
//! Persiste um histórico cronológico das comunicações dos agentes (contexto
//! roteado, pedidos ask e respostas reply) por workspace, para que a atividade
//! sobreviva a reinícios e possa ser consultada/reproduzida (replay = leitura
//! ordenada do log). O output bruto do terminal NÃO entra aqui — é mantido no
//! scrollback do terminal e no barramento em memória.
//!
//! Modelo: log anexado e limitado por workspace (evita crescimento sem fim),
//! isolado por `workspace_id`.

use crate::state::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;

/// Limite de entradas de histórico mantidas por workspace.
pub const HISTORY_LIMIT: usize = 1000;

/// Entrada de histórico de comunicação de um agente.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub workspace_id: String,
    pub agent_id: String,
    /// Direção/ex-tipo do evento (context_out, context_in, ask_out, etc.).
    pub direction: String,
    pub data: String,
    pub timestamp: u64,
}

/// Anexa uma entrada ao histórico persistido do workspace, mantendo o limite.
/// Falhas de disco não derrubam a execução (log é best-effort).
pub fn append(
    persistence: &crate::persistence::Persistence,
    workspace_id: &str,
    agent_id: &str,
    direction: &str,
    data: &str,
) {
    let entry = HistoryEntry {
        id: crate::workflow::gen_id("hist"),
        workspace_id: workspace_id.to_string(),
        agent_id: agent_id.to_string(),
        direction: direction.to_string(),
        data: data.to_string(),
        timestamp: crate::workflow::now_millis(),
    };

    let mut entries = persistence.load_history(workspace_id).unwrap_or_default();
    entries.push(entry);
    if entries.len() > HISTORY_LIMIT {
        let excess = entries.len() - HISTORY_LIMIT;
        entries.drain(0..excess);
    }
    let _ = persistence.save_history(workspace_id, &entries);
}

/// Lista o histórico de comunicação do workspace atual (ordenado por tempo).
#[tauri::command]
pub fn list_history(state: State<'_, AppState>) -> Result<Vec<HistoryEntry>, String> {
    let ws = {
        let guard = state.workspace.lock().expect("workspace mutex poisoned");
        guard.metadata.id.clone()
    };
    let mut entries = state.persistence.load_history(&ws)?;
    entries.sort_by_key(|e| e.timestamp);
    Ok(entries)
}

/// Histórico de comunicação de um agente no workspace atual.
#[tauri::command]
pub fn history_by_agent(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<Vec<HistoryEntry>, String> {
    let ws = {
        let guard = state.workspace.lock().expect("workspace mutex poisoned");
        guard.metadata.id.clone()
    };
    let mut entries: Vec<HistoryEntry> = state
        .persistence
        .load_history(&ws)?
        .into_iter()
        .filter(|e| e.agent_id == agent_id)
        .collect();
    entries.sort_by_key(|e| e.timestamp);
    Ok(entries)
}

/// Limpa o histórico persistido do workspace atual.
#[tauri::command]
pub fn clear_history(state: State<'_, AppState>) -> Result<(), String> {
    let ws = {
        let guard = state.workspace.lock().expect("workspace mutex poisoned");
        guard.metadata.id.clone()
    };
    state.persistence.save_history(&ws, &[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::Persistence;

    fn temp_dir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "maestri-history-test-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn append_persists_and_is_ordered() {
        let p = Persistence::new(temp_dir());
        append(&p, "ws-a", "a1", "ask_out", "ping");
        append(&p, "ws-a", "a2", "reply_out", "pong");

        let all = p.load_history("ws-a").unwrap();
        assert_eq!(all.len(), 2);
        assert!(all[0].timestamp <= all[1].timestamp);
    }

    #[test]
    fn history_is_bounded() {
        let p = Persistence::new(temp_dir());
        let mut entries = Vec::new();
        for i in 0..HISTORY_LIMIT {
            entries.push(HistoryEntry {
                id: format!("h{i}"),
                workspace_id: "ws-a".to_string(),
                agent_id: "a1".to_string(),
                direction: "context_out".to_string(),
                data: format!("m{i}"),
                timestamp: i as u64,
            });
        }
        p.save_history("ws-a", &entries).unwrap();

        append(&p, "ws-a", "a1", "context_out", "overflow");
        let all = p.load_history("ws-a").unwrap();
        // Mantém o limite: remove os mais antigos.
        assert_eq!(all.len(), HISTORY_LIMIT);
        assert_eq!(all.last().unwrap().data, "overflow");
    }

    #[test]
    fn history_isolated_by_workspace() {
        let p = Persistence::new(temp_dir());
        append(&p, "ws-a", "a1", "context_out", "in-a");
        append(&p, "ws-b", "a1", "context_out", "in-b");

        let a = p.load_history("ws-a").unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].data, "in-a");
    }
}