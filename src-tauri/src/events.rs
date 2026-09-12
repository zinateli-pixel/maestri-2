//! Camada de eventos observáveis do Workflow Engine (Fase 7).
//!
//! Eventos tipados (workflow_started, step_started, message_delivered,
//! step_failed, workflow_completed, workflow_failed), com scopo de
//! `workspace_id` + `workflow_id`, armazenados num log recuperável por
//! workspace (nunca vaza entre workspaces).

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// Tipos de evento do workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowEventType {
    WorkflowStarted,
    StepStarted,
    MessageDelivered,
    StepFailed,
    WorkflowCompleted,
    WorkflowFailed,
}

impl WorkflowEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkflowEventType::WorkflowStarted => "workflow_started",
            WorkflowEventType::StepStarted => "step_started",
            WorkflowEventType::MessageDelivered => "message_delivered",
            WorkflowEventType::StepFailed => "step_failed",
            WorkflowEventType::WorkflowCompleted => "workflow_completed",
            WorkflowEventType::WorkflowFailed => "workflow_failed",
        }
    }
}

/// Evento observável de workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEvent {
    pub id: String,
    pub workspace_id: String,
    pub workflow_id: String,
    pub event: WorkflowEventType,
    /// Agente associado (destino da mensagem, ou o step), quando aplicável.
    pub agent_id: Option<String>,
    /// Id do step (agente) quando aplicável.
    pub step: Option<String>,
    /// Payload/erro associado.
    pub data: Option<String>,
    pub timestamp: u64,
}

/// Log de eventos em memória, com limite e recuperação por workspace.
#[derive(Debug, Default)]
pub struct WorkflowEventLog {
    events: Mutex<Vec<WorkflowEvent>>,
}

const EVENT_LOG_CAP: usize = 2000;

impl WorkflowEventLog {
    pub fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }

    pub fn push(&self, event: WorkflowEvent) {
        let mut guard = self.events.lock().expect("workflow events mutex poisoned");
        guard.push(event);
        if guard.len() > EVENT_LOG_CAP {
            let excess = guard.len() - EVENT_LOG_CAP;
            guard.drain(0..excess);
        }
    }

    /// Adiciona vários eventos de uma vez (hidratação a partir do disco).
    pub fn extend(&self, events: Vec<WorkflowEvent>) {
        let mut guard = self.events.lock().expect("workflow events mutex poisoned");
        guard.extend(events);
        if guard.len() > EVENT_LOG_CAP {
            let excess = guard.len() - EVENT_LOG_CAP;
            guard.drain(0..excess);
        }
    }

    /// Eventos de um workspace (isolamento: nunca mistura workspaces).
    pub fn by_workspace(&self, workspace_id: &str) -> Vec<WorkflowEvent> {
        self.events
            .lock()
            .expect("workflow events mutex poisoned")
            .iter()
            .filter(|e| e.workspace_id == workspace_id)
            .cloned()
            .collect()
    }

    /// Eventos de um workspace + workflow específico.
    pub fn by_workflow(&self, workspace_id: &str, workflow_id: &str) -> Vec<WorkflowEvent> {
        self.events
            .lock()
            .expect("workflow events mutex poisoned")
            .iter()
            .filter(|e| e.workspace_id == workspace_id && e.workflow_id == workflow_id)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evt(workspace_id: &str, workflow_id: &str, event: WorkflowEventType) -> WorkflowEvent {
        WorkflowEvent {
            id: format!("{}-{}", workspace_id, event.as_str()),
            workspace_id: workspace_id.to_string(),
            workflow_id: workflow_id.to_string(),
            event,
            agent_id: None,
            step: None,
            data: None,
            timestamp: 0,
        }
    }

    #[test]
    fn log_scopes_events_by_workspace() {
        let log = WorkflowEventLog::new();
        log.push(evt("ws-A", "w1", WorkflowEventType::WorkflowStarted));
        log.push(evt("ws-A", "w1", WorkflowEventType::WorkflowCompleted));
        log.push(evt("ws-B", "w2", WorkflowEventType::WorkflowStarted));

        // ws-A só vê os próprios eventos.
        let a = log.by_workspace("ws-A");
        assert_eq!(a.len(), 2);
        assert!(a.iter().all(|e| e.workspace_id == "ws-A"));

        // ws-B vê apenas o dele.
        let b = log.by_workspace("ws-B");
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].workflow_id, "w2");

        // por workflow + workspace.
        let a_w1 = log.by_workflow("ws-A", "w1");
        assert_eq!(a_w1.len(), 2);
    }
}