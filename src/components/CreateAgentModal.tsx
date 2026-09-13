import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useWorkspaceStore } from "../store/workspaceStore";
import { AgentForm } from "./AgentForm";
import type { CreateAgentInput, RuntimeInfo } from "../types/models";

interface CreateAgentModalProps {
  onClose: () => void;
}

/** Templates de início rápido (preenchem apenas defaults — editáveis). */
const TEMPLATES: Array<{
  id: string;
  label: string;
  description: string;
  role: CreateAgentInput["role"];
  command: string;
  args: string[];
  runtime: CreateAgentInput["runtime"];
  model: string;
}> = [
  {
    id: "kilo-orchestrator",
    label: "Kilo Orchestrator",
    description: "Orquestra outros agents, planeja e delega tarefas",
    role: "orchestrator",
    command: "kilo",
    args: [],
    runtime: "kilo",
    model: "Kilo",
  },
  {
    id: "claude-specialist",
    label: "Claude Specialist",
    description: "Especialista em tarefas complexas de código e análise",
    role: "specialist",
    command: "claude",
    args: [],
    runtime: "claude_code",
    model: "Claude 3.5 Sonnet",
  },
  {
    id: "kilo-auditor",
    label: "Kilo Auditor",
    description: "Audita código, revisa segurança e qualidade",
    role: "auditor",
    command: "kilo",
    args: [],
    runtime: "kilo",
    model: "Kilo",
  },
  {
    id: "claude-builder",
    label: "Claude Builder",
    description: "Constrói features, escreve testes e documentação",
    role: "builder",
    command: "claude",
    args: [],
    runtime: "claude_code",
    model: "Claude 3.5 Sonnet",
  },
  {
    id: "opencode-reviewer",
    label: "OpenCode Reviewer",
    description: "Revisa PRs, sugere melhorias e detecta bugs",
    role: "reviewer",
    command: "opencode",
    args: [],
    runtime: "opencode",
    model: "OpenCode",
  },
  {
    id: "ollama-qa",
    label: "Ollama QA",
    description: "Testa, valida e garante qualidade local",
    role: "qa",
    command: "ollama",
    args: ["run", "qwen3:8b"],
    runtime: "custom",
    model: "qwen3:8b",
  },
  {
    id: "shell",
    label: "Shell",
    description: "Terminal genérico para comandos arbitrários",
    role: "observer",
    command: "zsh",
    args: [],
    runtime: "custom",
    model: "",
  },
  {
    id: "custom",
    label: "Custom Agent",
    description: "Configure tudo manualmente",
    role: "observer",
    command: "",
    args: [],
    runtime: "custom",
    model: "",
  },
];

export function CreateAgentModal({ onClose }: CreateAgentModalProps) {
  const createAgent = useWorkspaceStore((s) => s.createAgent);
  const [runtimes, setRuntimes] = useState<RuntimeInfo[]>([]);
  const [initial, setInitial] = useState<Partial<CreateAgentInput> | undefined>(
    undefined
  );
  const [selectedTemplate, setSelectedTemplate] = useState<string | null>(null);

  // Detecta runtimes disponíveis no sistema (sem instalar nada).
  useEffect(() => {
    invoke<RuntimeInfo[]>("detect_runtimes")
      .then(setRuntimes)
      .catch(() => setRuntimes([]));
  }, []);

  const handleCreate = async (input: CreateAgentInput) => {
    await createAgent(input);
    onClose();
  };

  const applyTemplate = (t: (typeof TEMPLATES)[number]) => {
    setSelectedTemplate(t.id);
    setInitial({
      name: t.label,
      role: t.role,
      command: t.command,
      args: t.args,
      runtime: t.runtime,
      model: t.model,
      working_dir: "",
      auto_start: false,
    });
  };

  const available = new Set(runtimes.filter((r) => r.available).map((r) => r.command));

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="panel-header">
          <h2>Novo Agent</h2>
          <button className="icon-btn" onClick={onClose} aria-label="Fechar">
            ✕
          </button>
        </div>

        {/* Início rápido — templates de runtime */}
        <div className="panel-section">
          <div className="panel-section-title">Início Rápido</div>
          <div style={{ display: "flex", gap: "var(--space-3)", flexWrap: "wrap" }}>
            {TEMPLATES.map((t) => {
              const isAvailable = t.command === "" || available.has(t.command);
              const isSelected = selectedTemplate === t.id;
              return (
                <button
                  key={t.id}
                  className="btn btn-ghost"
                  onClick={() => applyTemplate(t)}
                  title={t.description}
                  style={{
                    display: "flex",
                    flexDirection: "column",
                    alignItems: "flex-start",
                    gap: "var(--space-1)",
                    padding: "var(--space-3) var(--space-4)",
                    minWidth: 140,
                    maxWidth: 180,
                    textAlign: "left",
                    border: isSelected
                      ? "2px solid var(--color-accent-primary)"
                      : "1px solid var(--color-border-default)",
                    background: isSelected
                      ? "var(--color-accent-primary-muted)"
                      : "transparent",
                    opacity: isAvailable ? 1 : 0.5,
                    transition: "all var(--transition-fast)",
                  }}
                >
                  <span style={{ fontSize: "var(--text-sm)", fontWeight: "var(--font-weight-semibold)" }}>
                    {t.label}
                  </span>
                  <span style={{ fontSize: "var(--text-xs)", color: "var(--color-text-subtle)" }}>
                    {t.description}
                  </span>
                  <span
                    style={{
                      fontSize: "var(--text-xs)",
                      color: isAvailable
                        ? "var(--color-accent-success)"
                        : "var(--color-text-subtle)",
                    }}
                  >
                    {isAvailable ? "● available" : "○ not installed"}
                  </span>
                </button>
              );
            })}
          </div>
        </div>

        <div className="panel-divider" />

        <AgentForm
          key={JSON.stringify(initial)}
          initial={initial}
          submitLabel="Criar agent"
          onSubmit={handleCreate}
          onCancel={onClose}
        />
      </div>
    </div>
  );
}
