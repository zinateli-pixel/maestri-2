import { useEffect, useMemo, useRef, useState } from "react";
import { useReactFlow } from "@xyflow/react";
import { save, open } from "@tauri-apps/plugin-dialog";
import { useWorkspaceStore } from "../store/workspaceStore";

interface CommandPaletteProps {
  onClose: () => void;
  onOpenConfig: (id: string) => void;
  onCenterNode: (id: string) => void;
}

interface PaletteItem {
  id: string;
  label: string;
  hint?: string;
  icon?: string;
  action: () => void;
}

export function CommandPalette({
  onClose,
  onOpenConfig,
  onCenterNode,
}: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const rf = useReactFlow();

  const store = useWorkspaceStore();
  const state = store.state;

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const items: PaletteItem[] = useMemo(() => {
    const list: PaletteItem[] = [];
    const agents = state?.agents ?? [];

    // Ações globais
    list.push({
      id: "new-agent",
      label: "Create Agent",
      hint: "⌘N",
      icon: "＋",
      action: () => {
        window.dispatchEvent(new CustomEvent("maestri:new-agent"));
        onClose();
      },
    });
    list.push({
      id: "save",
      label: "Save Workspace",
      hint: "⌘S",
      icon: "💾",
      action: () => {
        void store.saveNow();
        onClose();
      },
    });
    list.push({
      id: "fit",
      label: "Fit View",
      hint: "F",
      icon: "⛶",
      action: () => {
        rf.fitView({ padding: 0.2, duration: 300 });
        onClose();
      },
    });
    list.push({
      id: "zoom-in",
      label: "Zoom In",
      hint: "⌘+",
      icon: "🔍＋",
      action: () => {
        void rf.zoomIn({ duration: 200 });
        onClose();
      },
    });
    list.push({
      id: "zoom-out",
      label: "Zoom Out",
      hint: "⌘−",
      icon: "🔍−",
      action: () => {
        void rf.zoomOut({ duration: 200 });
        onClose();
      },
    });
    list.push({
      id: "undo",
      label: "Undo",
      hint: "⌘Z",
      icon: "↶",
      action: () => {
        void store.undo();
        onClose();
      },
    });
    list.push({
      id: "redo",
      label: "Redo",
      hint: "⌘⇧Z",
      icon: "↷",
      action: () => {
        void store.redo();
        onClose();
      },
    });
    list.push({
      id: "export",
      label: "Export Workspace",
      icon: "📤",
      action: () => {
        void (async () => {
          const json = await store.exportWorkspace();
          const path = await save({
            defaultPath: "workspace.json",
            filters: [{ name: "JSON", extensions: ["json"] }],
          });
          if (path) {
            const { writeTextFile } = await import("@tauri-apps/plugin-fs");
            await writeTextFile(path, json);
            store.pushToast("Workspace exportado", "success");
          }
          onClose();
        })();
      },
    });
    list.push({
      id: "import",
      label: "Import Workspace",
      icon: "📥",
      action: () => {
        void (async () => {
          const path = await open({
            filters: [{ name: "JSON", extensions: ["json"] }],
          });
          if (path && typeof path === "string") {
            const { readTextFile } = await import("@tauri-apps/plugin-fs");
            const json = await readTextFile(path);
            if (window.confirm("Substituir o workspace atual?")) {
              await store.importWorkspace(json);
            }
          }
          onClose();
        })();
      },
    });
    list.push({
      id: "backup",
      label: "Backup Workspace",
      icon: "🗄",
      action: () => {
        void store.backupWorkspace().catch((e) => store.pushToast(String(e), "error"));
        onClose();
      },
    });
    list.push({
      id: "auto-layout",
      label: "Auto Layout (Grid)",
      hint: "⌘L",
      icon: "⊞",
      action: () => {
        window.dispatchEvent(new CustomEvent("maestri:auto-layout"));
        onClose();
      },
    });
    list.push({
      id: "select-all",
      label: "Select All Agents",
      hint: "⌘A",
      icon: "☐",
      action: () => {
        window.dispatchEvent(new CustomEvent("maestri:select-all"));
        onClose();
      },
    });
    list.push({
      id: "settings",
      label: "Workspace Settings",
      icon: "⚙",
      action: () => {
        window.dispatchEvent(new CustomEvent("maestri:open-settings"));
        onClose();
      },
    });
    list.push({
      id: "activity",
      label: "Toggle Activity Panel",
      icon: "☰",
      action: () => {
        window.dispatchEvent(new CustomEvent("maestri:toggle-activity"));
        onClose();
      },
    });

    // Ações por agente
    for (const agent of agents) {
      list.push({
        id: `goto-${agent.id}`,
        label: `Go to: ${agent.name}`,
        hint: `${agent.role} · ${agent.runtime}`,
        icon: "🎯",
        action: () => onCenterNode(agent.id),
      });
      list.push({
        id: `config-${agent.id}`,
        label: `Configure: ${agent.name}`,
        icon: "⚙",
        action: () => onOpenConfig(agent.id),
      });
      list.push({
        id: `refresh-${agent.id}`,
        label: `Refresh: ${agent.name}`,
        icon: "↻",
        action: () => {
          void store.refreshAgent(agent.id);
          onClose();
        },
      });
      list.push({
        id: `restart-${agent.id}`,
        label: `Restart: ${agent.name}`,
        icon: "↻",
        action: () => {
          void store.restartAgent(agent.id);
          onClose();
        },
      });
      list.push({
        id: `duplicate-${agent.id}`,
        label: `Duplicate: ${agent.name}`,
        icon: "⧉",
        action: () => {
          void store.duplicateAgent(agent.id);
          onClose();
        },
      });
      list.push({
        id: `delete-${agent.id}`,
        label: `Delete: ${agent.name}`,
        icon: "🗑",
        action: () => {
          void store.deleteAgent(agent.id);
          onClose();
        },
      });
    }

    return list;
  }, [state, store, rf, onClose, onOpenConfig, onCenterNode]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return items;
    return items.filter((i) => i.label.toLowerCase().includes(q));
  }, [items, query]);

  useEffect(() => {
    setIndex(0);
  }, [query]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      onClose();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setIndex((i) => Math.min(filtered.length - 1, i + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setIndex((i) => Math.max(0, i - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const item = filtered[index];
      if (item) item.action();
    }
  };

  return (
    <div className="command-palette" onClick={onClose}>
      <div className="command-palette-inner" onClick={(e) => e.stopPropagation()}>
        <input
          ref={inputRef}
          className="command-palette-input"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder="Buscar comando ou agente…"
        />
        <ul className="command-palette-list" role="listbox">
          {filtered.length === 0 && (
            <li style={{ padding: "var(--space-4)", color: "var(--color-text-subtle)", fontSize: "var(--text-sm)", textAlign: "center" }}>
              Nenhum resultado
            </li>
          )}
          {filtered.map((item, i) => (
            <li
              key={item.id}
              className={`command-palette-item ${i === index ? "selected" : ""}`}
              onClick={item.action}
              onMouseEnter={() => setIndex(i)}
              role="option"
              aria-selected={i === index}
            >
              <span className="command-palette-item-icon">{item.icon}</span>
              <div className="command-palette-item-content">
                <span className="command-palette-item-title">{item.label}</span>
              </div>
              {item.hint && (
                <span className="command-palette-item-shortcut">{item.hint}</span>
              )}
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
