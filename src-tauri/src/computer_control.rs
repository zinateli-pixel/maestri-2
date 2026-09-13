//! Fase 16 — Computer Control.
//!
//! Define o catálogo de ações de controle do computador (navegador/aplicativo)
//! que um agente pode solicitar, com a execução SEMPRE mediada pela camada de
//! permissões da Fase 17 (deny por padrão).
//!
//! O driver real (automação de navegador/app) é uma integração futura que
//! exigirá dependências externas (Playwright/XCTest/adb...). Aqui fica o
//! CONTRATO: ação → permissão requerida → resultado, com um executor stub que
//! já valida a autorização e retorna um resultado claro e observável.

use crate::permissions::{Permission, PermissionRegistry};
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;

/// Tipo de ação de controle do computador.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerActionKind {
    /// Navegar para uma URL (navegador).
    Navigate,
    /// Clicar num seletor/coordenada (navegador).
    Click,
    /// Digitar texto (navegador).
    TypeText,
    /// Capturar a tela (navegador).
    Screenshot,
    /// Lançar um aplicativo (desktop/mobile).
    LaunchApp,
    /// Tocar/ativar um elemento do aplicativo.
    Tap,
}

impl ComputerActionKind {
    /// Permissão exigida para executar a ação.
    pub fn required_permission(&self) -> Permission {
        match self {
            ComputerActionKind::LaunchApp | ComputerActionKind::Tap => Permission::AppControl,
            ComputerActionKind::Navigate
            | ComputerActionKind::Click
            | ComputerActionKind::TypeText
            | ComputerActionKind::Screenshot => Permission::BrowserControl,
        }
    }
}

/// Ação completa: tipo + alvo (URL, seletor, texto, nome do app...).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ComputerAction {
    pub action: ComputerActionKind,
    pub target: String,
}

/// Resultado da execução de uma ação de controle do computador.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ComputerActionResult {
    /// Se a permissão exigida foi concedida.
    pub permitted: bool,
    /// Se a ação foi de fato executada (false enquanto não há driver real).
    pub executed: bool,
    /// Mensagem explicativa para o agente/operador.
    pub message: String,
}

fn denied(action: &ComputerAction) -> ComputerActionResult {
    ComputerActionResult {
        permitted: false,
        executed: false,
        message: format!(
            "permissão '{}' não concedida para a ação '{:?}'",
            action.action.required_permission().as_str(),
            action.action
        ),
    }
}

/// Despacha uma ação de controle do computador, verificando a permissão do
/// agente no workspace. Sem driver real, uma ação autorizada ainda retorna
/// `executed = false` (Fase 16: contrato; driver na integração futura).
pub fn dispatch(
    permissions: &PermissionRegistry,
    workspace_id: &str,
    agent_id: &str,
    action: &ComputerAction,
) -> ComputerActionResult {
    let required = action.action.required_permission();
    if !permissions.is_granted(workspace_id, agent_id, required) {
        return denied(action);
    }

    ComputerActionResult {
        permitted: true,
        executed: false,
        message: format!(
            "ação '{:?}' autorizada, mas o driver de controle ainda não está integrado",
            action.action
        ),
    }
}

/// Comando Tauri: solicita uma ação de controle do computador para um agente.
/// Resolve o workspace atual e valida a existência do agente.
#[tauri::command]
pub fn computer_action(
    state: State<'_, AppState>,
    agent_id: String,
    action: ComputerAction,
) -> Result<ComputerActionResult, String> {
    let (workspace_id, agent_exists) = {
        let guard = state
            .workspace
            .lock()
            .expect("workspace mutex poisoned");
        let exists = guard.agents.iter().any(|a| a.id == agent_id);
        (guard.metadata.id.clone(), exists)
    };

    if !agent_exists {
        return Err("agente não encontrado no workspace".to_string());
    }

    Ok(dispatch(&state.permissions, &workspace_id, &agent_id, &action))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::PermissionRegistry;

    fn click(target: &str) -> ComputerAction {
        ComputerAction {
            action: ComputerActionKind::Click,
            target: target.to_string(),
        }
    }

    #[test]
    fn action_kind_maps_to_permission() {
        assert_eq!(
            ComputerActionKind::Navigate.required_permission(),
            Permission::BrowserControl
        );
        assert_eq!(
            ComputerActionKind::Click.required_permission(),
            Permission::BrowserControl
        );
        assert_eq!(
            ComputerActionKind::LaunchApp.required_permission(),
            Permission::AppControl
        );
        assert_eq!(
            ComputerActionKind::Tap.required_permission(),
            Permission::AppControl
        );
    }

    #[test]
    fn browser_action_denied_without_permission() {
        let reg = PermissionRegistry::new();
        let result = dispatch(&reg, "ws-a", "a1", &click("#submit"));
        assert!(!result.permitted);
        assert!(!result.executed);
        assert!(result.message.contains("não concedida"));
    }

    #[test]
    fn browser_action_authorized_but_stub_driver() {
        let reg = PermissionRegistry::new();
        reg.grant("ws-a", "a1", Permission::BrowserControl);
        let result = dispatch(&reg, "ws-a", "a1", &click("#submit"));
        assert!(result.permitted);
        // Sem driver real (stub Fase 16), a ação ainda não executa.
        assert!(!result.executed);
        assert!(result.message.contains("driver"));
    }

    #[test]
    fn app_permission_does_not_authorize_browser_action() {
        let reg = PermissionRegistry::new();
        reg.grant("ws-a", "a1", Permission::AppControl);
        // BrowserControl é distinto de AppControl: clique continua negado.
        let result = dispatch(&reg, "ws-a", "a1", &click("#submit"));
        assert!(!result.permitted);
    }

    #[test]
    fn result_serde_snake_case() {
        let r = ComputerActionResult {
            permitted: true,
            executed: false,
            message: "x".to_string(),
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"permitted\""));
        assert!(json.contains("\"executed\""));
    }
}