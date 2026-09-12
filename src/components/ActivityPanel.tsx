import { useEffect, useRef } from "react";
import { useWorkspaceStore } from "../store/workspaceStore";

interface ActivityPanelProps {
  onClose: () => void;
}

const KIND_COLORS: Record<string, string> = {
  info: "var(--color-accent-info)",
  success: "var(--color-accent-success)",
  error: "var(--color-accent-error)",
};

function formatTime(ts: number): string {
  const d = new Date(ts);
  return d.toLocaleTimeString("pt-BR", { hour12: false });
}

export function ActivityPanel({ onClose }: ActivityPanelProps) {
  const activity = useWorkspaceStore((s) => s.activity);
  const clearActivity = useWorkspaceStore((s) => s.clearActivity);
  const listRef = useRef<HTMLDivElement>(null);

  // Auto-scroll para o fim quando novas entradas chegam.
  useEffect(() => {
    const el = listRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [activity.length]);

  return (
    <aside className="panel" style={{ width: 320 }}>
      <div className="panel-header">
        <h2>Activity</h2>
        <div style={{ display: "flex", gap: "var(--space-2)" }}>
          <button
            className="icon-btn"
            onClick={clearActivity}
            title="Limpar histórico"
            aria-label="Limpar histórico"
          >
            🗑
          </button>
          <button className="icon-btn" onClick={onClose} aria-label="Fechar">
            ✕
          </button>
        </div>
      </div>

      <div
        ref={listRef}
        style={{
          flex: 1,
          overflowY: "auto",
          display: "flex",
          flexDirection: "column",
          gap: "var(--space-2)",
          minHeight: 0,
        }}
      >
        {activity.length === 0 && (
          <div
            style={{
              color: "var(--color-text-subtle)",
              fontSize: "var(--text-sm)",
              textAlign: "center",
              padding: "var(--space-6)",
            }}
          >
            Nenhuma atividade nesta sessão.
          </div>
        )}
        {activity.map((entry) => (
          <div
            key={entry.id}
            style={{
              display: "flex",
              gap: "var(--space-3)",
              padding: "var(--space-2) var(--space-3)",
              background: "var(--color-bg-base)",
              borderRadius: "var(--radius-md)",
              borderLeft: `3px solid ${KIND_COLORS[entry.kind] ?? KIND_COLORS.info}`,
              fontSize: "var(--text-xs)",
            }}
          >
            <span
              style={{
                color: "var(--color-text-subtle)",
                fontFamily: "var(--font-mono)",
                flexShrink: 0,
              }}
            >
              {formatTime(entry.ts)}
            </span>
            <span style={{ color: "var(--color-text-secondary)", wordBreak: "break-word" }}>
              {entry.message}
            </span>
          </div>
        ))}
      </div>
    </aside>
  );
}
