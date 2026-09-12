//! Skills/capacidades de agentes (Fase 9) — primeira camada, agnóstica ao
//! runtime. Declara capacidades associáveis a agentes, escopadas ao workspace
//! (isolamento) e persistidas junto do `WorkspaceState`.
//!
//! Fora de escopo: executar skills, MCP, ferramentas externas, Memory,
//! Supabase/cloud, Web/App, Marketplace, scheduler avançado.

use crate::models::Skill;
use crate::state::AppState;
use serde::Deserialize;
use tauri::State;

/// Payload de criação de skill (versão e agente opcionais).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateSkillInput {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub version: Option<String>,
    /// Opcional: id do agente a associar já na criação.
    #[serde(default)]
    pub agent_id: Option<String>,
}

/// Lista as skills do workspace atual (catálogo).
#[tauri::command]
pub fn list_skills(state: State<'_, AppState>) -> Vec<Skill> {
    let guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    guard.skills.clone()
}

/// Consulta as skills associadas a um agente do workspace atual.
/// Um `agent_id` de outro workspace não resolve (isolamento).
#[tauri::command]
pub fn skills_of_agent(state: State<'_, AppState>, agent_id: String) -> Vec<Skill> {
    let guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    if !guard.agents.iter().any(|a| a.id == agent_id) {
        return Vec::new();
    }
    guard.skills_of(&agent_id)
}

/// Cria uma skill no workspace atual (e opcionalmente associa a um agente).
#[tauri::command]
pub fn create_skill(
    state: State<'_, AppState>,
    input: CreateSkillInput,
) -> Result<Skill, String> {
    if input.name.trim().is_empty() {
        return Err("nome da skill é obrigatório".to_string());
    }

    let skill = Skill {
        id: crate::workflow::gen_id("skill"),
        name: input.name.trim().to_string(),
        description: input.description.trim().to_string(),
        version: input.version,
    };

    {
        let mut guard = state
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        if let Some(agent_id) = &input.agent_id {
            if !guard.agents.iter().any(|a| &a.id == agent_id) {
                return Err("agente não encontrado no workspace".to_string());
            }
        }
        guard.skills.push(skill.clone());
        if let Some(agent_id) = &input.agent_id {
            guard.associate_skill(agent_id, &skill.id);
        }
    }

    state.persist()?;
    Ok(skill)
}

/// Associa uma skill existente a um agente do workspace atual.
#[tauri::command]
pub fn associate_skill(
    state: State<'_, AppState>,
    agent_id: String,
    skill_id: String,
) -> Result<(), String> {
    let mut guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    if !guard.agents.iter().any(|a| a.id == agent_id) {
        return Err("agente não encontrado no workspace".to_string());
    }
    if guard.get_skill(&skill_id).is_none() {
        return Err("skill não encontrada no workspace".to_string());
    }
    guard.associate_skill(&agent_id, &skill_id);
    drop(guard);
    state.persist()
}

/// Remove uma skill do workspace (e das associações de agentes).
#[tauri::command]
pub fn delete_skill(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut guard = state
        .workspace
        .lock()
        .expect("workspace mutex poisoned");
    let before = guard.skills.len();
    guard.skills.retain(|s| s.id != id);
    if guard.skills.len() == before {
        return Err("skill não encontrada".to_string());
    }
    for ids in guard.agent_skills.values_mut() {
        ids.retain(|s| s != &id);
    }
    drop(guard);
    state.persist()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Agent, AgentKind, Role, Runtime, Skill, Status, WorkspaceState};

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

    fn skill(id: &str, name: &str) -> Skill {
        Skill {
            id: id.to_string(),
            name: name.to_string(),
            description: "d".to_string(),
            version: None,
        }
    }

    #[test]
    fn skill_serializes_with_and_without_version() {
        let s = skill("s1", "Review");
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("version"));
        assert_eq!(serde_json::from_str::<Skill>(&json).unwrap(), s);

        let v = Skill {
            id: "s2".into(),
            name: "X".into(),
            description: "y".into(),
            version: Some("1.0".into()),
        };
        let vjson = serde_json::to_string(&v).unwrap();
        assert!(vjson.contains("\"version\":\"1.0\""));
        assert_eq!(serde_json::from_str::<Skill>(&vjson).unwrap(), v);
    }

    #[test]
    fn association_and_query_by_agent() {
        let mut ws = WorkspaceState::new("ws".into());
        ws.skills = vec![skill("s1", "Review"), skill("s2", "Test")];
        ws.associate_skill("a1", "s1");
        ws.associate_skill("a1", "s2");

        let got = ws.skills_of("a1");
        assert_eq!(got.len(), 2);
        assert!(got.iter().any(|s| s.id == "s1"));
        assert!(got.iter().any(|s| s.id == "s2"));

        // Idempotente: reassociar não duplica.
        assert!(!ws.associate_skill("a1", "s1"));
        assert_eq!(ws.skills_of("a1").len(), 2);
    }

    #[test]
    fn skills_are_isolated_between_workspaces() {
        let mut ws_a = WorkspaceState::new("A".into());
        ws_a.agents.push(agent("a1"));
        ws_a.skills = vec![skill("s1", "OnlyA")];
        ws_a.associate_skill("a1", "s1");

        let ws_b = WorkspaceState::new("B".into()); // sem skills

        assert_eq!(ws_a.skills_of("a1").len(), 1);
        assert!(ws_b.skills_of("a1").is_empty());
        assert!(!ws_b.skills.iter().any(|s| s.id == "s1"));
    }

    #[test]
    fn nonexistent_skill_returns_empty_or_none() {
        let mut ws = WorkspaceState::new("ws".into());
        ws.agents.push(agent("a1"));

        assert!(ws.get_skill("ghost").is_none());
        assert!(ws.skills_of("a1").is_empty());
    }
}