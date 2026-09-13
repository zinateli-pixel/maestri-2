import { useState } from "react";
import { useWorkspaceStore } from "../store/workspaceStore";
import { AgentForm } from "./AgentForm";
import type { Agent, CreateAgentInput } from "../types/models";

interface AgentDetailPanelProps {
  agent: Agent;
  onClose: () => void;
}

const ROLE_LABELS: Record<string, string> = {
  orchestrator: "Orchestrator",
  specialist: "Specialist",
  auditor: "Auditor",
  builder: "Builder",
  reviewer: "Reviewer",
  qa: "QA",
  observer: "Observer",
};

const STATUS_LABELS: Record<string, string> = {
  idle: "STOPPED",
  starting: "STARTING",
  running: "RUNNING",
  waiting: "WAITING",
  done: "DONE",
  failed: "ERROR",
  stopped: "STOPPED",
};

const STATUS_COLORS: Record<string, string> = {
  idle: "var(--color-status-idle)",
  starting: "var(--color-status-starting)",
  running: "var(--color-status-running)",
  waiting: "var(--color-status-waiting)",
  done: "var(--color-status-done)",
  failed: "var(--color-status-failed)",
  stopped: "var(--color-status-stopped)",
};

export function AgentDetailPanel({ agent, onClose }: AgentDetailPanelProps) {
  const updateAgent = useWorkspaceStore((s) => s.updateAgent);
  const deleteAgent = useWorkspaceStore((s) => s.deleteAgent);

  const [confirming, setConfirming] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const handleUpdate = async (input: CreateAgentInput) => {
    await updateAgent({ id: agent.id, ...input });
    onClose();
  };

  const handleDelete = async () => {
    setDeleting(true);
    try {
      await deleteAgent(agent.id);
      onClose();
    } catch (e) {
      setDeleting(false);
      alert(String(e));
    }
  };

  const statusColor = STATUS_COLORS[agent.status] ?? "var(--color-text-subtle)";
  const roleColor = agent.accent ?? "var(--color-text-subtle)";

  return (
    <aside className="panel">
      <div className="panel-header">
        <div style={{ display: "flex", alignItems: "center", gap: "var(--space-3)" }}>
          <span
            style={{
              width: 10,
              height: 10,
              borderRadius: "var(--radius-full)",
              background: statusColor,
              boxShadow: agent.status === "running" ? `0 0 8px ${statusColor}` : "none",
            }}
          />
          <h2 style={{ margin: 0 }}>{agent.name}</h2>
        </div>
        <button className="icon-btn" onClick={onClose} aria-label="Fechar">
          ✕
        </button>
      </div>

      {/* Identity section */}
      <div className="panel-section">
        <div className="panel-section-title">Identity</div>
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-3)" }}>
          <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
            <span
              style={{
                fontSize: "var(--text-xs)",
                fontWeight: "var(--font-weight-semibold)",
                color: roleColor,
                textTransform: "uppercase",
                letterSpacing: "0.5px",
              }}
            >
              {ROLE_LABELS[agent.role] ?? agent.role.toUpperCase()}
            </span>
            <span style={{ fontSize: "var(--text-xs)", color: "var(--color-text-subtle)" }}>
              {agent.runtime}
              {agent.model ? ` · ${agent.model}` : ""}
            </span>
          </div>
          <div style={{ fontSize: "var(--text-xs)", color: "var(--color-text-subtle)", fontFamily: "var(--font-mono)" }}>
            ID: {agent.id}
          </div>
        </div>
      </div>

      <div className="panel-divider" />

      {/* Runtime section */}
      <div className="panel-section">
        <div className="panel-section-title">Runtime</div>
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-3)" }}>
          <div>
            <span style={{ fontSize: "var(--text-xs)", color: "var(--color-text-subtle)" }}>Command</span>
            <div style={{ fontFamily: "var(--font-mono)", fontSize: "var(--text-sm)" }}>
              {agent.command} {agent.args.join(" ")}
            </div>
          </div>
          <div>
            <span style={{ fontSize: "var(--text-xs)", color: "var(--color-text-subtle)" }}>Working Directory</span>
            <div style={{ fontFamily: "var(--font-mono)", fontSize: "var(--text-sm)", color: "var(--color-text-muted)" }}>
              {agent.working_dir || "(default)"}
            </div>
          </div>
          <div>
            <span style={{ fontSize: "var(--text-xs)", color: "var(--color-text-subtle)" }}>Auto Start</span>
            <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
              <input
                type="checkbox"
                checked={agent.auto_start}
                disabled
                style={{ width: 16, height: 16, accentColor: "var(--color-accent-primary)" }}
              />
              <span style={{ fontSize: "var(--text-sm)", color: agent.auto_start ? "var(--color-accent-success)" : "var(--color-text-subtle)" }}>
                {agent.auto_start ? "Enabled" : "Disabled"}
              </span>
            </div>
          </div>
        </div>
      </div>

      <div className="panel-divider" />

      {/* Status section */}
      <div className="panel-section">
        <div className="panel-section-title">Status</div>
        <div style={{ display: "flex", alignItems: "center", gap: "var(--space-3)" }}>
          <span
            className="status-dot"
            style={{
              width: 10,
              height: 10,
              borderRadius: "var(--radius-full)",
              background: statusColor,
              boxShadow: agent.status === "running" ? `0 0 8px ${statusColor}` : "none",
            }}
          />
          <span style={{ fontWeight: "var(--font-weight-medium)" }}>
            {STATUS_LABELS[agent.status] ?? agent.status}
          </span>
          {agent.locked && (
            <span title="Locked" style={{ color: "var(--color-text-subtle)" }}>🔒</span>
          )}
        </div>
      </div>

      <div className="panel-divider" />

      {/* Form for editing */}
      <AgentForm
        initial={{
          name: agent.name,
          role: agent.role,
          runtime: agent.runtime,
          kind: agent.kind,
          model: agent.model,
          command: agent.command,
          args: agent.args,
          working_dir: agent.working_dir,
        }}
        submitLabel="Salvar alterações"
        onSubmit={handleUpdate}
        onCancel={onClose}
      />

      <div className="panel-divider" />

      {/* Danger zone */}
      <div className="panel-section" style={{ marginTop: "auto" }}>
        <div className="panel-section-title" style={{ color: "var(--color-accent-error)" }}>Danger Zone</div>
        {confirming ? (
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-3)" }}>
            <div style={{ color: "var(--color-accent-error)", fontSize: "var(--text-sm)" }}>
              Excluir “{agent.name}”?
            </div>
            <div style={{ display: "flex", gap: "var(--space-2)", justifyContent: "flex-end" }}>
              <button
                className="btn btn-ghost"
                onClick={() => setConfirming(false)}
                disabled={deleting}
              >
                Cancelar
              </button>
              <button
                className="btn btn-danger"
                onClick={handleDelete}
                disabled={deleting}
              >
                {deleting ? "Excluindo…" : "Excluir"}
              </button>
            </div>
          </div>
        ) : (
          <button
            className="btn btn-danger"
            onClick={() => setConfirming(true)}
            style={{ width: "100%" }}
          >
            Excluir agent
          </button>
        )}
      </div>
    </aside>
  );
}