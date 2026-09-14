//! Fase 19 — Templates.
//!
//! Templates são presets reutilizáveis de agente (role, runtime, kind, modelo,
//! comando, args, working_dir, auto_start) que o operador pode criar, listar,
//! excluir e aplicar — criando um agente novo no workspace atual a partir do
//! template. Persistidos globalmente (não por workspace), pois são reutilizáveis
//! entre workspaces.

use crate::models::{Agent, AgentKind, Role, Runtime, Status};
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;

/// Template reutilizável de agente.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub role: Role,
    pub runtime: Runtime,
    pub kind: AgentKind,
    pub model: String,
    pub command: String,
    pub args: Vec<String>,
    pub working_dir: String,
    pub auto_start: bool,
}

/// Payload de criação de template.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateTemplateInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub role: Role,
    pub runtime: Runtime,
    #[serde(default)]
    pub kind: AgentKind,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub working_dir: String,
    #[serde(default)]
    pub auto_start: bool,
}

/// Lista todos os templates persistidos.
#[tauri::command]
pub fn list_templates(state: State<'_, AppState>) -> Result<Vec<AgentTemplate>, String> {
    state.persistence.load_templates()
}

/// Cria e persiste um template.
#[tauri::command]
pub fn create_template(
    state: State<'_, AppState>,
    input: CreateTemplateInput,
) -> Result<AgentTemplate, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("nome do template é obrigatório".to_string());
    }

    let template = AgentTemplate {
        id: crate::workflow::gen_id("tpl"),
        name,
        description: input.description.trim().to_string(),
        role: input.role,
        runtime: input.runtime,
        kind: input.kind,
        model: input.model.trim().to_string(),
        command: input.command.trim().to_string(),
        args: input.args,
        working_dir: input.working_dir.trim().to_string(),
        auto_start: input.auto_start,
    };

    let mut all = state.persistence.load_templates()?;
    all.push(template.clone());
    state.persistence.save_templates(&all)?;
    Ok(template)
}

/// Remove um template pelo id.
#[tauri::command]
pub fn delete_template(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut all = state.persistence.load_templates()?;
    let before = all.len();
    all.retain(|t| t.id != id);
    if all.len() == before {
        return Err("template não encontrado".to_string());
    }
    state.persistence.save_templates(&all)
}

/// Aplica um template: cria um agente novo no workspace atual com as
/// configurações do template (posição default, status Idle).
#[tauri::command]
pub fn apply_template(
    state: State<'_, AppState>,
    template_id: String,
) -> Result<Agent, String> {
    let template = state
        .persistence
        .load_templates()?
        .into_iter()
        .find(|t| t.id == template_id)
        .ok_or_else(|| "template não encontrado".to_string())?;

    let agent = Agent {
        id: crate::workflow::gen_id("agent"),
        name: template.name.clone(),
        role: template.role,
        runtime: template.runtime,
        kind: template.kind,
        model: template.model.clone(),
        command: template.command.clone(),
        args: template.args.clone(),
        working_dir: template.working_dir.clone(),
        status: Status::Idle,
        x: 0.5,
        y: 0.5,
        width: None,
        height: None,
        collapsed: false,
        locked: false,
        accent: None,
        auto_start: template.auto_start,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AgentTemplate {
        AgentTemplate {
            id: "t1".to_string(),
            name: "Kilo Builder".to_string(),
            description: String::new(),
            role: Role::Builder,
            runtime: Runtime::Kilo,
            kind: AgentKind::Cli,
            model: "kilo".to_string(),
            command: "kilo".to_string(),
            args: vec![],
            working_dir: String::new(),
            auto_start: false,
        }
    }

    #[test]
    fn template_serializes_roundtrip() {
        let t = sample();
        let json = serde_json::to_string(&t).unwrap();
        let back: AgentTemplate = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn template_serde_snake_case_fields() {
        let json = serde_json::to_string(&sample()).unwrap();
        assert!(json.contains("\"auto_start\""));
        assert!(json.contains("\"working_dir\""));
    }
}