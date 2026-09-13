import { useCallback, useEffect, useState } from "react";
import { useWorkspaceStore } from "../store/workspaceStore";
import type {
  ChainExecution,
  McpServer,
  McpTransport,
  MemoryEntry,
  Project,
  Skill,
  WorkflowEvent,
} from "../types/models";

type Tab = "workflow" | "skills" | "mcp" | "memory" | "projects";

const EVENT_LABEL: Record<string, string> = {
  workflow_started: "iniciado",
  step_started: "passo iniciado",
  message_delivered: "mensagem entregue",
  step_failed: "passo falhou",
  workflow_completed: "concluído",
  workflow_failed: "falhou",
};

function fmt(ts: number): string {
  return new Date(ts).toLocaleTimeString("pt-BR", { hour12: false });
}

export function CapabilitiesPanel({ onClose }: { onClose: () => void }) {
  const agents = useWorkspaceStore((s) => s.state?.agents ?? []);
  const currentWorkspaceId = useWorkspaceStore((s) => s.currentWorkspaceId);
  const runChain = useWorkspaceStore((s) => s.runChain);
  const listWorkflowEvents = useWorkspaceStore((s) => s.listWorkflowEvents);
  const listSkills = useWorkspaceStore((s) => s.listSkills);
  const createSkill = useWorkspaceStore((s) => s.createSkill);
  const associateSkill = useWorkspaceStore((s) => s.associateSkill);
  const deleteSkill = useWorkspaceStore((s) => s.deleteSkill);
  const listMcpServers = useWorkspaceStore((s) => s.listMcpServers);
  const createMcpServer = useWorkspaceStore((s) => s.createMcpServer);
  const associateMcpToAgent = useWorkspaceStore((s) => s.associateMcpToAgent);
  const deleteMcpServer = useWorkspaceStore((s) => s.deleteMcpServer);
  const listMemory = useWorkspaceStore((s) => s.listMemory);
  const createMemory = useWorkspaceStore((s) => s.createMemory);
  const removeMemory = useWorkspaceStore((s) => s.removeMemory);
  const listProjects = useWorkspaceStore((s) => s.listProjects);
  const createProject = useWorkspaceStore((s) => s.createProject);
  const renameProject = useWorkspaceStore((s) => s.renameProject);
  const deleteProject = useWorkspaceStore((s) => s.deleteProject);
  const addWorkspaceToProject = useWorkspaceStore((s) => s.addWorkspaceToProject);
  const removeWorkspaceFromProject = useWorkspaceStore((s) => s.removeWorkspaceFromProject);

  const [tab, setTab] = useState<Tab>("workflow");

  // Workflow
  const [payload, setPayload] = useState("");
  const [lastRun, setLastRun] = useState<ChainExecution | null>(null);
  const [events, setEvents] = useState<WorkflowEvent[]>([]);

  // Skills
  const [skills, setSkills] = useState<Skill[]>([]);
  const [skillName, setSkillName] = useState("");
  const [skillDesc, setSkillDesc] = useState("");
  const [skillVersion, setSkillVersion] = useState("");
  const [skillAgentId, setSkillAgentId] = useState("");
  const [assocSkillId, setAssocSkillId] = useState("");
  const [assocSkillAgentId, setAssocSkillAgentId] = useState("");

  // MCP
  const [mcps, setMcps] = useState<McpServer[]>([]);
  const [mcpName, setMcpName] = useState("");
  const [mcpTransport, setMcpTransport] = useState<McpTransport>("stdio");
  const [mcpCommand, setMcpCommand] = useState("");
  const [mcpUrl, setMcpUrl] = useState("");
  const [mcpEnabled, setMcpEnabled] = useState(true);
  const [mcpAgentId, setMcpAgentId] = useState("");
  const [assocMcpId, setAssocMcpId] = useState("");
  const [assocMcpAgentId, setAssocMcpAgentId] = useState("");

  // Memory
  const [memories, setMemories] = useState<MemoryEntry[]>([]);
  const [memCategory, setMemCategory] = useState("");
  const [memContent, setMemContent] = useState("");
  const [memAgentId, setMemAgentId] = useState("");

  // Projects (Fase 12)
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectName, setProjectName] = useState("");

  const refresh = useCallback(async () => {
    try {
      const [s, m, e, mm, pj] = await Promise.all([
        listSkills(),
        listMcpServers(),
        listWorkflowEvents(),
        listMemory(),
        listProjects(),
      ]);
      setSkills(s);
      setMcps(m);
      setEvents(e);
      setMemories(mm);
      setProjects(pj);
    } catch (e) {
      console.error("CapabilitiesPanel refresh:", e);
    }
  }, [listSkills, listMcpServers, listWorkflowEvents, listMemory, listProjects]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const run = async () => {
    const exec = await runChain(payload.trim() || "tarefa");
    setLastRun(exec);
    await refresh();
  };

  const addSkill = async () => {
    if (!skillName.trim()) return;
    await createSkill({
      name: skillName.trim(),
      description: skillDesc.trim(),
      version: skillVersion.trim() || undefined,
      agent_id: skillAgentId || undefined,
    });
    setSkillName("");
    setSkillDesc("");
    setSkillVersion("");
    setSkillAgentId("");
    await refresh();
  };

  const addMcp = async () => {
    if (!mcpName.trim()) return;
    await createMcpServer({
      name: mcpName.trim(),
      transport: mcpTransport,
      command: mcpTransport === "stdio" ? mcpCommand.trim() || undefined : undefined,
      url: mcpTransport !== "stdio" ? mcpUrl.trim() || undefined : undefined,
      enabled: mcpEnabled,
      agent_id: mcpAgentId || undefined,
    });
    setMcpName("");
    setMcpCommand("");
    setMcpUrl("");
    setMcpAgentId("");
    await refresh();
  };

  const addMemory = async () => {
    if (!memCategory.trim() || !memContent.trim()) return;
    await createMemory({
      category: memCategory.trim(),
      content: memContent.trim(),
      agent_id: memAgentId || undefined,
    });
    setMemCategory("");
    setMemContent("");
    setMemAgentId("");
    await refresh();
  };

  return (
    <aside className="panel" style={{ width: 380 }}>
      <div className="panel-header">
        <h2>Capabilities</h2>
        <button className="icon-btn" onClick={onClose} aria-label="Fechar">
          ✕
        </button>
      </div>

      <div style={{ display: "flex", gap: "var(--space-2)", padding: "0 var(--space-3)" }}>
        {(["workflow", "skills", "mcp", "memory", "projects"] as Tab[]).map((t) => (
          <button
            key={t}
            className={t === tab ? "btn btn-primary btn-sm" : "btn btn-ghost btn-sm"}
            onClick={() => setTab(t)}
          >
            {t === "workflow" ? "Workflow" : t === "skills" ? "Skills" : t === "mcp" ? "MCP" : t === "memory" ? "Memory" : "Projects"}
          </button>
        ))}
      </div>

      <div
        style={{
          flex: 1,
          overflowY: "auto",
          minHeight: 0,
          padding: "var(--space-3)",
          display: "flex",
          flexDirection: "column",
          gap: "var(--space-3)",
        }}
      >
        {tab === "workflow" && (
          <>
            <label className="field">
              <span className="field-label">Tarefa (payload)</span>
              <input
                className="field-input"
                value={payload}
                onChange={(e) => setPayload(e.target.value)}
                placeholder="mensagem/tarefa do fluxo"
              />
            </label>
            <button className="btn btn-primary" onClick={run}>
              ▶ Executar A→B→C (canvas)
            </button>

            {lastRun && (
              <div style={{ fontSize: "var(--text-sm)" }}>
                <div>
                  Status:{" "}
                  <span style={{ fontWeight: 600 }}>{lastRun.status}</span>
                </div>
                <div style={{ color: "var(--color-text-subtle)" }}>
                  Ordem: {lastRun.order.join(" → ") || "nenhuma"}
                </div>
                {lastRun.error && (
                  <div style={{ color: "var(--color-accent-error)" }}>
                    {lastRun.error}
                  </div>
                )}
              </div>
            )}

            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                alignItems: "center",
              }}
            >
              <span className="panel-section-title">Eventos do workflow</span>
              <button className="icon-btn" onClick={refresh} title="Atualizar">
                ↻
              </button>
            </div>
            {events.length === 0 && (
              <div style={{ color: "var(--color-text-subtle)", fontSize: "var(--text-xs)" }}>
                Nenhum evento ainda.
              </div>
            )}
            {events.slice(-40).reverse().map((e) => (
              <div
                key={e.id}
                style={{
                  fontSize: "var(--text-xs)",
                  padding: "var(--space-2) var(--space-3)",
                  background: "var(--color-bg-base)",
                  borderRadius: "var(--radius-md)",
                }}
              >
                <span style={{ color: "var(--color-text-subtle)", fontFamily: "var(--font-mono)" }}>
                  {fmt(e.timestamp)}
                </span>{" "}
                <span style={{ fontWeight: 600 }}>{EVENT_LABEL[e.event] ?? e.event}</span>
                {e.agent_id && (
                  <span style={{ color: "var(--color-text-secondary)" }}> · {e.agent_id}</span>
                )}
              </div>
            ))}
          </>
        )}

        {tab === "skills" && (
          <>
            <div className="panel-section-title">Nova skill</div>
            <label className="field">
              <span className="field-label">Nome</span>
              <input className="field-input" value={skillName} onChange={(e) => setSkillName(e.target.value)} />
            </label>
            <label className="field">
              <span className="field-label">Descrição</span>
              <input className="field-input" value={skillDesc} onChange={(e) => setSkillDesc(e.target.value)} />
            </label>
            <label className="field">
              <span className="field-label">Versão (opcional)</span>
              <input className="field-input" value={skillVersion} onChange={(e) => setSkillVersion(e.target.value)} />
            </label>
            <label className="field">
              <span className="field-label">Associar a (opcional)</span>
              <select
                className="field-input"
                value={skillAgentId}
                onChange={(e) => setSkillAgentId(e.target.value)}
              >
                <option value="">— nenhum —</option>
                {agents.map((a) => (
                  <option key={a.id} value={a.id}>{a.name}</option>
                ))}
              </select>
            </label>
            <button className="btn btn-primary" onClick={addSkill}>＋ Criar skill</button>

            <div className="panel-divider" />
            <div className="panel-section-title">Associar skill a agente</div>
            <div style={{ display: "flex", gap: "var(--space-2)" }}>
              <select
                className="field-input"
                value={assocSkillAgentId}
                onChange={(e) => setAssocSkillAgentId(e.target.value)}
                style={{ flex: 1 }}
              >
                <option value="">Agente…</option>
                {agents.map((a) => (
                  <option key={a.id} value={a.id}>{a.name}</option>
                ))}
              </select>
              <select
                className="field-input"
                value={assocSkillId}
                onChange={(e) => setAssocSkillId(e.target.value)}
                style={{ flex: 1 }}
              >
                <option value="">Skill…</option>
                {skills.map((s) => (
                  <option key={s.id} value={s.id}>{s.name}</option>
                ))}
              </select>
              <button
                className="btn btn-ghost"
                onClick={() =>
                  assocSkillAgentId && assocSkillId
                    ? associateSkill(assocSkillAgentId, assocSkillId).then(refresh)
                    : undefined
                }
              >
                Vincular
              </button>
            </div>

            <div className="panel-divider" />
            <div className="panel-section-title">Skills ({skills.length})</div>
            {skills.map((s) => (
              <div
                key={s.id}
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "center",
                  padding: "var(--space-2) var(--space-3)",
                  background: "var(--color-bg-base)",
                  borderRadius: "var(--radius-md)",
                  fontSize: "var(--text-sm)",
                }}
              >
                <div>
                  <div style={{ fontWeight: 600 }}>{s.name}</div>
                  <div style={{ color: "var(--color-text-subtle)", fontSize: "var(--text-xs)" }}>
                    {s.description}
                    {s.version ? ` · v${s.version}` : ""}
                  </div>
                </div>
                <button className="icon-btn" onClick={() => deleteSkill(s.id).then(refresh)} title="Remover">
                  🗑
                </button>
              </div>
            ))}
          </>
        )}

        {tab === "mcp" && (
          <>
            <div className="panel-section-title">Novo MCP server</div>
            <label className="field">
              <span className="field-label">Nome</span>
              <input className="field-input" value={mcpName} onChange={(e) => setMcpName(e.target.value)} />
            </label>
            <label className="field">
              <span className="field-label">Transporte</span>
              <select
                className="field-input"
                value={mcpTransport}
                onChange={(e) => setMcpTransport(e.target.value as McpTransport)}
              >
                <option value="stdio">stdio</option>
                <option value="http">http</option>
                <option value="sse">sse</option>
              </select>
            </label>
            {mcpTransport === "stdio" ? (
              <label className="field">
                <span className="field-label">Comando</span>
                <input className="field-input" value={mcpCommand} onChange={(e) => setMcpCommand(e.target.value)} placeholder="ex.: node server.js" />
              </label>
            ) : (
              <label className="field">
                <span className="field-label">URL</span>
                <input className="field-input" value={mcpUrl} onChange={(e) => setMcpUrl(e.target.value)} placeholder="http://…" />
              </label>
            )}
            <label className="field" style={{ flexDirection: "row", alignItems: "center", gap: "var(--space-3)" }}>
              <input
                type="checkbox"
                checked={mcpEnabled}
                onChange={(e) => setMcpEnabled(e.target.checked)}
                style={{ width: 16, height: 16 }}
              />
              <span style={{ fontSize: "var(--text-sm)" }}>Habilitado</span>
            </label>
            <label className="field">
              <span className="field-label">Associar a agente (opcional)</span>
              <select className="field-input" value={mcpAgentId} onChange={(e) => setMcpAgentId(e.target.value)}>
                <option value="">— nenhum —</option>
                {agents.map((a) => (
                  <option key={a.id} value={a.id}>{a.name}</option>
                ))}
              </select>
            </label>
            <button className="btn btn-primary" onClick={addMcp}>＋ Criar server</button>

            <div className="panel-divider" />
            <div className="panel-section-title">Associar server a agente</div>
            <div style={{ display: "flex", gap: "var(--space-2)" }}>
              <select className="field-input" value={assocMcpAgentId} onChange={(e) => setAssocMcpAgentId(e.target.value)} style={{ flex: 1 }}>
                <option value="">Agente…</option>
                {agents.map((a) => (
                  <option key={a.id} value={a.id}>{a.name}</option>
                ))}
              </select>
              <select className="field-input" value={assocMcpId} onChange={(e) => setAssocMcpId(e.target.value)} style={{ flex: 1 }}>
                <option value="">Server…</option>
                {mcps.map((m) => (
                  <option key={m.id} value={m.id}>{m.name}</option>
                ))}
              </select>
              <button
                className="btn btn-ghost"
                onClick={() =>
                  assocMcpAgentId && assocMcpId
                    ? associateMcpToAgent(assocMcpAgentId, assocMcpId).then(refresh)
                    : undefined
                }
              >
                Vincular
              </button>
            </div>

            <div className="panel-divider" />
            <div className="panel-section-title">MCP Servers ({mcps.length})</div>
            {mcps.map((m) => (
              <div
                key={m.id}
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "center",
                  padding: "var(--space-2) var(--space-3)",
                  background: "var(--color-bg-base)",
                  borderRadius: "var(--radius-md)",
                  fontSize: "var(--text-sm)",
                }}
              >
                <div>
                  <div style={{ fontWeight: 600 }}>
                    {m.name}{" "}
                    <span
                      style={{
                        color: m.enabled ? "var(--color-accent-success)" : "var(--color-text-subtle)",
                        fontSize: "var(--text-xs)",
                      }}
                    >
                      {m.enabled ? "on" : "off"}
                    </span>
                  </div>
                  <div style={{ color: "var(--color-text-subtle)", fontSize: "var(--text-xs)" }}>
                    {m.transport}
                    {m.config?.command ? ` · ${m.config.command}` : ""}
                    {m.config?.url ? ` · ${m.config.url}` : ""}
                  </div>
                </div>
                <button className="icon-btn" onClick={() => deleteMcpServer(m.id).then(refresh)} title="Remover">
                  🗑
                </button>
              </div>
            ))}
          </>
        )}

        {tab === "memory" && (
          <>
            <div className="panel-section-title">Nova memória</div>
            <label className="field">
              <span className="field-label">Categoria</span>
              <input className="field-input" value={memCategory} onChange={(e) => setMemCategory(e.target.value)} placeholder="ex.: fato, preferência, contexto" />
            </label>
            <label className="field">
              <span className="field-label">Conteúdo</span>
              <textarea
                className="field-input"
                value={memContent}
                onChange={(e) => setMemContent(e.target.value)}
                placeholder="lembrar que…"
                rows={3}
                style={{ resize: "vertical" }}
              />
            </label>
            <label className="field">
              <span className="field-label">Agente (opcional; vazio = compartilhada)</span>
              <select className="field-input" value={memAgentId} onChange={(e) => setMemAgentId(e.target.value)}>
                <option value="">— compartilhada —</option>
                {agents.map((a) => (
                  <option key={a.id} value={a.id}>{a.name}</option>
                ))}
              </select>
            </label>
            <button className="btn btn-primary" onClick={addMemory}>＋ Criar memória</button>

            <div className="panel-divider" />
            <div className="panel-section-title">Memórias ({memories.length})</div>
            {memories.length === 0 && (
              <div style={{ color: "var(--color-text-subtle)", fontSize: "var(--text-xs)" }}>
                Nenhuma memória ainda.
              </div>
            )}
            {memories.map((m) => (
              <div
                key={m.id}
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "center",
                  gap: "var(--space-2)",
                  padding: "var(--space-2) var(--space-3)",
                  background: "var(--color-bg-base)",
                  borderRadius: "var(--radius-md)",
                  fontSize: "var(--text-sm)",
                }}
              >
                <div style={{ minWidth: 0 }}>
                  <div style={{ fontWeight: 600 }}>
                    <span style={{ color: "var(--color-text-subtle)", fontSize: "var(--text-xs)" }}>
                      {m.category}
                    </span>{" "}
                    {m.agent_id ? (
                      <span style={{ color: "var(--color-accent-primary)", fontSize: "var(--text-xs)" }}>
                        · {m.agent_id}
                      </span>
                    ) : (
                      <span style={{ color: "var(--color-accent-success)", fontSize: "var(--text-xs)" }}>
                        · compartilhada
                      </span>
                    )}
                  </div>
                  <div style={{ color: "var(--color-text-secondary)", wordBreak: "break-word" }}>
                    {m.content}
                  </div>
                </div>
                <button className="icon-btn" onClick={() => removeMemory(m.id).then(refresh)} title="Remover">
                  🗑
                </button>
              </div>
            ))}
          </>
        )}

        {tab === "projects" && (
          <>
            <div className="panel-section-title">Novo projeto</div>
            <label className="field">
              <span className="field-label">Nome</span>
              <input
                className="field-input"
                value={projectName}
                onChange={(e) => setProjectName(e.target.value)}
                placeholder="ex.: Cliente X"
              />
            </label>
            <button
              className="btn btn-primary"
              onClick={async () => {
                if (!projectName.trim()) return;
                await createProject({ name: projectName.trim() });
                setProjectName("");
                await refresh();
              }}
            >
              ＋ Criar projeto
            </button>

            <div className="panel-divider" />
            <div className="panel-section-title">Projetos ({projects.length})</div>
            {projects.length === 0 && (
              <div style={{ color: "var(--color-text-subtle)", fontSize: "var(--text-xs)" }}>
                Nenhum projeto ainda.
              </div>
            )}
            {projects.map((p) => (
              <div
                key={p.id}
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "center",
                  gap: "var(--space-2)",
                  padding: "var(--space-2) var(--space-3)",
                  background: "var(--color-bg-base)",
                  borderRadius: "var(--radius-md)",
                  fontSize: "var(--text-sm)",
                }}
              >
                <div style={{ minWidth: 0 }}>
                  <div style={{ fontWeight: 600 }}>{p.name}</div>
                  <div style={{ color: "var(--color-text-subtle)", fontSize: "var(--text-xs)" }}>
                    {p.workspace_ids.length} workspace(s)
                  </div>
                </div>
                <div style={{ display: "flex", gap: "var(--space-2)", alignItems: "center" }}>
                  {currentWorkspaceId &&
                    (p.workspace_ids.includes(currentWorkspaceId) ? (
                      <button
                        className="icon-btn"
                        title="Remover workspace atual do projeto"
                        onClick={() =>
                          removeWorkspaceFromProject({
                            project_id: p.id,
                            workspace_id: currentWorkspaceId,
                          }).then(refresh)
                        }
                      >
                        ➖
                      </button>
                    ) : (
                      <button
                        className="icon-btn"
                        title="Associar workspace atual ao projeto"
                        onClick={() =>
                          addWorkspaceToProject({
                            project_id: p.id,
                            workspace_id: currentWorkspaceId,
                          }).then(refresh)
                        }
                      >
                        ＋
                      </button>
                    ))}
                  <button
                    className="icon-btn"
                    title="Renomear projeto"
                    onClick={() => {
                      const name =
                        typeof window.prompt === "function"
                          ? window.prompt("Novo nome:", p.name ?? "")
                          : "";
                      if (name?.trim())
                        renameProject({ id: p.id, name: name.trim() })
                          .then(refresh)
                          .catch((err) => console.error(err));
                    }}
                  >
                    ✏️
                  </button>
                  <button className="icon-btn" title="Remover projeto" onClick={() => deleteProject(p.id).then(refresh)}>
                    🗑
                  </button>
                </div>
              </div>
            ))}
          </>
        )}
      </div>
    </aside>
  );
}