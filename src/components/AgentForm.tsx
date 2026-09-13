import { useState } from "react";
import type { CreateAgentInput, Role, Runtime } from "../types/models";

const ROLES: Role[] = [
  "orchestrator",
  "specialist",
  "auditor",
  "builder",
  "reviewer",
  "qa",
  "observer",
];

const RUNTIMES: Runtime[] = ["kilo", "claude_code", "opencode", "custom"];

const ROLE_LABELS: Record<Role, string> = {
  orchestrator: "Orchestrator",
  specialist: "Specialist",
  auditor: "Auditor",
  builder: "Builder",
  reviewer: "Reviewer",
  qa: "QA",
  observer: "Observer",
};

const RUNTIME_LABELS: Record<Runtime, string> = {
  kilo: "Kilo",
  claude_code: "Claude Code",
  opencode: "OpenCode",
  custom: "Custom",
};

const RUNTIME_COMMANDS: Partial<Record<Runtime, string>> = {
  kilo: "kilo",
  claude_code: "claude",
  opencode: "opencode",
};

const GENERIC_SHELLS = new Set(["", "sh", "bash", "zsh", "fish"]);

interface AgentFormProps {
  initial?: Partial<CreateAgentInput>;
  submitLabel: string;
  onSubmit: (input: CreateAgentInput) => Promise<void>;
  onCancel?: () => void;
}

export function AgentForm({
  initial,
  submitLabel,
  onSubmit,
  onCancel,
}: AgentFormProps) {
  const [name, setName] = useState(initial?.name ?? "");
  const [role, setRole] = useState<Role>(initial?.role ?? "builder");
  const [runtime, setRuntime] = useState<Runtime>(
    initial?.runtime ?? "kilo"
  );
  const [model, setModel] = useState(initial?.model ?? "");
  const [command, setCommand] = useState(initial?.command ?? "");
  const [args, setArgs] = useState(initial?.args?.join(" ") ?? "");
  const [workingDir, setWorkingDir] = useState(
    initial?.working_dir ?? ""
  );
  const [autoStart, setAutoStart] = useState(initial?.auto_start ?? false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) {
      setError("Nome é obrigatório.");
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      await onSubmit({
        name: name.trim(),
        role,
        runtime,
        model: model.trim(),
        command: command.trim(),
        args: args
          .split(/\s+/)
          .map((a) => a.trim())
          .filter(Boolean),
        working_dir: workingDir.trim(),
        auto_start: autoStart,
      });
    } catch (err) {
      setError(String(err));
      setSubmitting(false);
    }
  };

  return (
    <form className="agent-form" onSubmit={handleSubmit}>
      {/* GENERAL */}
      <div className="panel-section">
        <div className="panel-section-title">General</div>

        <label className="field">
          <span className="field-label">Nome</span>
          <input
            className="field-input"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Ex.: Auditor de código"
            autoFocus
          />
        </label>

        <div className="field-row">
          <label className="field">
            <span className="field-label">Role</span>
            <select
              className="field-select"
              value={role}
              onChange={(e) => setRole(e.target.value as Role)}
            >
              {ROLES.map((r) => (
                <option key={r} value={r}>
                  {ROLE_LABELS[r]}
                </option>
              ))}
            </select>
          </label>

          <label className="field">
            <span className="field-label">Runtime</span>
            <select
              className="field-select"
              value={runtime}
              onChange={(e) => {
                const next = e.target.value as Runtime;
                setRuntime(next);
                const defaultCommand = RUNTIME_COMMANDS[next];
                if (defaultCommand && GENERIC_SHELLS.has(command.trim())) {
                  setCommand(defaultCommand);
                }
              }}
            >
              {RUNTIMES.map((r) => (
                <option key={r} value={r}>
                  {RUNTIME_LABELS[r]}
                </option>
              ))}
            </select>
          </label>
        </div>
      </div>

      {/* RUNTIME */}
      <div className="panel-section">
        <div className="panel-section-title">Runtime</div>

        <label className="field">
          <span className="field-label">Modelo</span>
          <input
            className="field-input"
            value={model}
            onChange={(e) => setModel(e.target.value)}
            placeholder="Ex.: claude-sonnet-4"
          />
        </label>

        <label className="field">
          <span className="field-label">Comando</span>
          <input
            className="field-input"
            value={command}
            onChange={(e) => setCommand(e.target.value)}
            placeholder="Ex.: claude"
          />
        </label>

        <label className="field">
          <span className="field-label">Argumentos (separados por espaço)</span>
          <input
            className="field-input"
            value={args}
            onChange={(e) => setArgs(e.target.value)}
            placeholder="Ex.: -p --dangerously-skip-permissions"
          />
        </label>
      </div>

      {/* TERMINAL */}
      <div className="panel-section">
        <div className="panel-section-title">Terminal</div>

        <label className="field">
          <span className="field-label">Diretório de trabalho</span>
          <input
            className="field-input"
            value={workingDir}
            onChange={(e) => setWorkingDir(e.target.value)}
            placeholder="Ex.: /Users/macbook/projeto"
          />
        </label>

        <label className="field">
          <span className="field-label">Iniciar automaticamente</span>
          <label style={{ display: "flex", alignItems: "center", gap: "var(--space-3)", cursor: "pointer" }}>
            <input
              type="checkbox"
              checked={autoStart}
              onChange={(e) => setAutoStart(e.target.checked)}
              style={{
                width: 16,
                height: 16,
                accentColor: "var(--color-accent-primary)",
              }}
            />
            <span style={{ fontSize: "var(--text-sm)", color: "var(--color-text-secondary)" }}>
              Iniciar este agente automaticamente ao abrir o workspace
            </span>
          </label>
        </label>
      </div>

      {error && <div className="field-error">{error}</div>}

      <div className="form-actions">
        {onCancel && (
          <button
            type="button"
            className="btn btn-ghost"
            onClick={onCancel}
            disabled={submitting}
          >
            Cancelar
          </button>
        )}
        <button type="submit" className="btn btn-primary" disabled={submitting}>
          {submitting ? "Salvando…" : submitLabel}
        </button>
      </div>
    </form>
  );
}
