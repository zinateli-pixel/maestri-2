import { useState, useRef, useEffect } from "react";
import { Handle, NodeResizer, Position, type NodeProps } from "@xyflow/react";
import { useWorkspaceStore } from "../../store/workspaceStore";
import { Terminal } from "../Terminal";
import type { Agent } from "../../types/models";

const ROLE_LABELS: Record<string, string> = {
  orchestrator: "ORCHESTRATOR",
  specialist: "SPECIALIST",
  auditor: "AUDITOR",
  builder: "BUILDER",
  reviewer: "REVIEWER",
  qa: "QA",
  observer: "OBSERVER",
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

const ROLE_COLORS: Record<string, string> = {
  orchestrator: "var(--color-role-orchestrator)",
  specialist: "var(--color-role-specialist)",
  auditor: "var(--color-role-auditor)",
  builder: "var(--color-role-builder)",
  reviewer: "var(--color-role-reviewer)",
  qa: "var(--color-role-qa)",
  observer: "var(--color-role-observer)",
};

export function AgentNode({ data, selected }: NodeProps) {
  const agent = data.agent as Agent;
  const onOpenConfig = data.onOpenConfig as (id: string) => void;
  const roleColor = agent.accent ?? ROLE_COLORS[agent.role] ?? "var(--color-text-subtle)";
  const statusColor = STATUS_COLORS[agent.status] ?? "var(--color-text-subtle)";

  const refreshAgent = useWorkspaceStore((s) => s.refreshAgent);
  const restartAgent = useWorkspaceStore((s) => s.restartAgent);
  const duplicateAgent = useWorkspaceStore((s) => s.duplicateAgent);
  const deleteAgent = useWorkspaceStore((s) => s.deleteAgent);
  const updateAgent = useWorkspaceStore((s) => s.updateAgent);
  const routeContext = useWorkspaceStore((s) => s.routeContext);

  const [menuOpen, setMenuOpen] = useState(false);
  const headerRef = useRef<HTMLDivElement>(null);

  const isActive = agent.status === "running" || agent.status === "starting";
  const isStarting = agent.status === "starting";
  const isError = agent.status === "failed";

  const run = (fn: () => Promise<unknown>) => (e: React.MouseEvent) => {
    e.stopPropagation();
    setMenuOpen(false);
    void fn().catch((err) => alert(String(err)));
  };

  const toggleCollapsed = run(async () => {
    await updateAgent({
      id: agent.id,
      name: agent.name,
      role: agent.role,
      runtime: agent.runtime,
      model: agent.model,
      command: agent.command,
      args: agent.args,
      working_dir: agent.working_dir,
      collapsed: !agent.collapsed,
    });
  });

  const toggleLocked = run(async () => {
    await updateAgent({
      id: agent.id,
      name: agent.name,
      role: agent.role,
      runtime: agent.runtime,
      model: agent.model,
      command: agent.command,
      args: agent.args,
      working_dir: agent.working_dir,
      locked: !agent.locked,
    });
  });

  const sendContext = run(async () => {
    if (typeof window.prompt !== "function") return;
    const payload = window.prompt(
      `Enviar contexto de "${agent.name}" para os agentes conectados:`,
      ""
    );
    if (!payload || !payload.trim()) return;
    await routeContext(agent.id, payload.trim());
  });

  // Close menu on outside click
  useEffect(() => {
    if (!menuOpen) return;
    const onDocDown = (ev: MouseEvent) => {
      if (headerRef.current && !headerRef.current.contains(ev.target as Node)) {
        setMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", onDocDown);
    return () => document.removeEventListener("mousedown", onDocDown);
  }, [menuOpen]);

  const borderColor = selected ? "var(--color-accent-primary)" : roleColor;
  const boxShadow = selected
    ? "0 0 0 1px var(--color-accent-primary), 0 8px 24px rgba(0,0,0,0.4)"
    : "0 4px 16px rgba(0,0,0,0.4)";

  return (
    <div
      ref={headerRef}
      style={{
        width: "100%",
        height: agent.collapsed ? "auto" : "100%",
        display: "flex",
        flexDirection: "column",
        border: `1px solid ${borderColor}`,
        borderRadius: "var(--radius-xl)",
        background: "var(--color-bg-elevated)",
        color: "var(--color-text-primary)",
        boxShadow,
        overflow: "hidden",
        opacity: agent.locked ? 0.85 : 1,
        transition: "box-shadow var(--transition-fast), border-color var(--transition-fast)",
      }}
    >
      <NodeResizer
        isVisible={!!selected && !agent.locked && !agent.collapsed}
        minWidth={360}
        minHeight={240}
        lineStyle={{ borderColor: "var(--color-accent-primary)" }}
        handleStyle={{ background: "var(--color-accent-primary)", width: 8, height: 8 }}
      />
      <Handle type="target" position={Position.Top} />

      {/* HEADER — compact, professional */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--space-3)",
          padding: "var(--space-2) var(--space-3)",
          borderBottom: agent.collapsed ? "none" : "1px solid var(--color-border-subtle)",
          cursor: agent.locked ? "default" : "grab",
          flexShrink: 0,
          minHeight: "36px",
        }}
      >
        {/* Status indicator */}
        <span
          className="status-dot"
          style={{
            width: 8,
            height: 8,
            borderRadius: "var(--radius-full)",
            background: statusColor,
            flexShrink: 0,
            boxShadow: isActive ? `0 0 8px ${statusColor}` : "none",
            transition: "box-shadow var(--transition-normal)",
          }}
          title={STATUS_LABELS[agent.status] ?? agent.status}
        />

        {/* Role badge */}
        <span
          style={{
            fontSize: "var(--text-xs)",
            fontWeight: "var(--font-weight-semibold)",
            color: roleColor,
            textTransform: "uppercase",
            letterSpacing: "0.5px",
            flexShrink: 0,
            whiteSpace: "nowrap",
          }}
        >
          {ROLE_LABELS[agent.role] ?? agent.role.toUpperCase()}
        </span>

        {/* Name */}
        <span
          style={{
            fontSize: "var(--text-sm)",
            fontWeight: "var(--font-weight-bold)",
            flex: 1,
            minWidth: 0,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {agent.name}
        </span>

        {/* Runtime + Model */}
        <span
          style={{
            fontSize: "var(--text-xs)",
            color: "var(--color-text-subtle)",
            flexShrink: 0,
            whiteSpace: "nowrap",
          }}
        >
          {agent.runtime}
          {agent.model ? ` · ${agent.model}` : ""}
        </span>

        {/* Collapse toggle */}
        <button
          className="nodrag icon-btn"
          onClick={toggleCollapsed}
          title={agent.collapsed ? "Expandir" : "Colapsar"}
          aria-label={agent.collapsed ? "Expandir" : "Colapsar"}
          style={{
            width: 28,
            height: 28,
            fontSize: 12,
            flexShrink: 0,
          }}
        >
          {agent.collapsed ? "▸" : "▾"}
        </button>

        {/* Config */}
        <button
          className="nodrag icon-btn"
          onClick={(e) => {
            e.stopPropagation();
            onOpenConfig(agent.id);
          }}
          title="Configurações"
          aria-label="Configurações"
        >
          ⚙
        </button>

        {/* Refresh — primary action */}
        <button
          className="nodrag icon-btn"
          onClick={run(() => refreshAgent(agent.id))}
          disabled={isStarting}
          title={isStarting ? "Iniciando..." : "Atualizar processo (Refresh)"}
          aria-label={isStarting ? "Iniciando..." : "Atualizar processo"}
          style={{
            color: isStarting ? "var(--color-text-subtle)" : "var(--color-accent-primary)",
            opacity: isStarting ? 0.6 : 1,
          }}
        >
          ↻
        </button>

        {/* Menu ⋯ */}
        <div className="nodrag" style={{ position: "relative", flexShrink: 0 }}>
          <button
            className="icon-btn"
            onClick={(e) => {
              e.stopPropagation();
              setMenuOpen((o) => !o);
            }}
            title="Ações"
            aria-label="Ações"
            aria-expanded={menuOpen}
            aria-haspopup="true"
          >
            ⋯
          </button>
          {menuOpen && (
            <div
              className="context-menu"
              style={{
                position: "absolute",
                right: 0,
                top: "calc(100% + var(--space-1))",
                minWidth: 160,
                zIndex: 30,
              }}
              role="menu"
            >
              <button
                className="context-menu-item"
                onClick={run(() => refreshAgent(agent.id))}
                disabled={isStarting}
                role="menuitem"
              >
                {isStarting ? "⏳ Atualizando..." : "↻ Refresh"}
              </button>
              <button
                className="context-menu-item"
                onClick={run(() => restartAgent(agent.id))}
                role="menuitem"
              >
                ↻ Restart
              </button>
              <button
                className="context-menu-item"
                onClick={sendContext}
                role="menuitem"
              >
                → Send Context
              </button>
              <button
                className="context-menu-item"
                onClick={run(() => duplicateAgent(agent.id))}
                role="menuitem"
              >
                ⧉ Duplicate
              </button>
              <button
                className="context-menu-item"
                onClick={toggleLocked}
                role="menuitem"
              >
                {agent.locked ? "🔓 Unlock" : "🔒 Lock"}
              </button>
              <button
                className="context-menu-item danger"
                onClick={run(() => deleteAgent(agent.id))}
                role="menuitem"
              >
                🗑 Delete
              </button>
            </div>
          )}
        </div>
      </div>

      {/* BODY — terminal real (oculto quando colapsado) */}
      {!agent.collapsed && (
        <div
          className="nodrag nopan"
          style={{
            flex: 1,
            minHeight: 0,
            position: "relative",
            background: "var(--color-bg-deep)",
          }}
        >
          <Terminal agentId={agent.id} />
          {!isActive && (
            <div
              style={{
                position: "absolute",
                inset: 0,
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                justifyContent: "center",
                background: "rgba(8,12,20,0.85)",
                color: "var(--color-text-subtle)",
                fontSize: "var(--text-sm)",
                pointerEvents: "none",
                gap: "var(--space-3)",
                padding: "var(--space-4)",
              }}
            >
              <span style={{ opacity: 0.6 }}>●</span>
              <span>
                {isError ? "Process error — press ↻ Refresh" : "Process stopped — press ↻ Refresh"}
              </span>
            </div>
          )}
        </div>
      )}

      {/* FOOTER — status bar */}
      {!agent.collapsed && (
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--space-3)",
            padding: "var(--space-2) var(--space-3)",
            borderTop: "1px solid var(--color-border-subtle)",
            fontSize: "var(--text-xs)",
            color: "var(--color-text-subtle)",
            flexShrink: 0,
            background: "var(--color-bg-base)",
          }}
        >
          <span
            className="status-dot"
            style={{
              width: 6,
              height: 6,
              borderRadius: "var(--radius-full)",
              background: statusColor,
              boxShadow: isActive ? `0 0 6px ${statusColor}` : "none",
            }}
          />
          <span style={{ fontWeight: "var(--font-weight-medium)", color: "var(--color-text-secondary)" }}>
            {STATUS_LABELS[agent.status] ?? agent.status}
          </span>
          {agent.locked && (
            <span title="Locked" style={{ color: "var(--color-text-subtle)" }}>🔒</span>
          )}
          <span style={{ marginLeft: "auto", fontFamily: "var(--font-mono)" }}>
            {isActive ? "PTY connected" : "PTY disconnected"}
          </span>
        </div>
      )}

      <Handle type="source" position={Position.Bottom} />
    </div>
  );
}
