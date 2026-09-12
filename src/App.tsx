import { useEffect, useRef, useState } from "react";
import { ReactFlowProvider } from "@xyflow/react";
import { Canvas } from "./components/canvas/Canvas";
import { CreateAgentModal } from "./components/CreateAgentModal";
import { SettingsPanel } from "./components/SettingsPanel";
import { ActivityPanel } from "./components/ActivityPanel";
import { CapabilitiesPanel } from "./components/CapabilitiesPanel";
import { useWorkspaceStore } from "./store/workspaceStore";
import type { WorkspaceListItem } from "./types/models";
import "./App.css";

function Toasts() {
  const toasts = useWorkspaceStore((s) => s.toasts);
  const dismiss = useWorkspaceStore((s) => s.dismissToast);
  if (toasts.length === 0) return null;
  return (
    <div className="toasts">
      {toasts.map((t) => (
        <div key={t.id} className={`toast toast-${t.kind}`} onClick={() => dismiss(t.id)}>
          {t.message}
        </div>
      ))}
    </div>
  );
}

function StatusBar() {
  const state = useWorkspaceStore((s) => s.state);
  const saveState = useWorkspaceStore((s) => s.saveState);
  if (!state) return null;
  const running = state.agents.filter(
    (a) => a.status === "running" || a.status === "starting"
  ).length;
  return (
    <footer className="status-bar">
      <span>Agents: {state.agents.length}</span>
      <span>Running: {running}</span>
      <span>Connections: {state.edges.length}</span>
      <span className="status-bar-right">
        {saveState === "saved" && "● Saved"}
        {saveState === "saving" && "● Saving…"}
        {saveState === "unsaved" && "● Unsaved"}
      </span>
    </footer>
  );
}

function ManageButton({ ws }: { ws: WorkspaceListItem }) {
  const [menuOpen, setMenuOpen] = useState(false);
  return (
    <span
      className="workspace-manage-wrap"
      onClick={(ev) => ev.stopPropagation()}
    >
      <button
        className="icon-btn workspace-manage-toggle"
        title="Gerenciar workspace"
        onClick={(ev) => {
          ev.stopPropagation();
          setMenuOpen(!menuOpen);
        }}
      >
        ⋮
      </button>
      {menuOpen && (
        <div className="workspace-menu-popover">
          <button
            className="popover-item"
            onClick={(ev) => {
              ev.stopPropagation();
              setMenuOpen(false);
              const name =
                typeof window.prompt === "function"
                  ? window.prompt("Novo nome:", ws.name ?? "")
                  : "";
              if (name?.trim())
                useWorkspaceStore.getState().renameWorkspace({ id: ws.id, name: name.trim() }).then(() => {}).catch((err) => alert(err));
            }}
          >
            ✏️ Renomear
          </button>
          <button
            className="popover-item"
            onClick={(ev) => {
              ev.stopPropagation();
              setMenuOpen(false);
              useWorkspaceStore.getState().duplicateWorkspace(ws.id).then(() => {}).catch((err) => alert(err));
            }}
          >
            📄 Duplicar
          </button>
          <button
            className="popover-item danger"
            onClick={(ev) => {
              ev.stopPropagation();
              setMenuOpen(false);
              if (typeof window.confirm === "function" &&
                  window.confirm(`Deletar workspace "${ws.name}"?\nEsta ação é irreversível.`)) {
                useWorkspaceStore.getState().deleteWorkspace(ws.id).then(() => {}).catch((err) => alert(err));
              }
            }}
          >
            🗑 Deletar
          </button>
        </div>
      )}
    </span>
  );
}

function WorkspaceSelector() {
  const workspaces = useWorkspaceStore((s) => s.workspaces);
  const currentWorkspaceId = useWorkspaceStore((s) => s.currentWorkspaceId);
  const loadWorkspace = useWorkspaceStore((s) => s.loadWorkspace);
  const createWorkspace = useWorkspaceStore((s) => s.createWorkspace);
  const [isOpen, setIsOpen] = useState(false);
  const [newWorkspaceName, setNewWorkspaceName] = useState("");
  const ref = useRef<HTMLDivElement>(null);

  const currentWorkspace = workspaces.find((w) => w.id === currentWorkspaceId);

  // Close dropdown on outside click / Esc
  useEffect(() => {
    if (!isOpen) return;
    const onDocDown = (ev: MouseEvent) => {
      if (ref.current && !ref.current.contains(ev.target as Node)) setIsOpen(false);
    };
    const onEsc = (ev: KeyboardEvent) => {
      if (ev.key === "Escape") setIsOpen(false);
    };
    document.addEventListener("mousedown", onDocDown);
    document.addEventListener("keydown", onEsc);
    return () => {
      document.removeEventListener("mousedown", onDocDown);
      document.removeEventListener("keydown", onEsc);
    };
  }, [isOpen]);

  const handleCreateWorkspace = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newWorkspaceName.trim()) return;
    try {
      await createWorkspace({ name: newWorkspaceName.trim() });
      setNewWorkspaceName("");
      setIsOpen(false);
    } catch (e) {
      alert(String(e));
    }
  };

  return (
    <div className="workspace-selector" ref={ref}>
      <button
        className="btn btn-ghost workspace-trigger"
        onClick={() => setIsOpen(!isOpen)}
        title="Alternar workspace"
      >
        {currentWorkspace ? currentWorkspace.name : "Sem workspace"}
        <span className="caret">▾</span>
      </button>
      {isOpen && (
        <div className="workspace-dropdown">
          <form onSubmit={handleCreateWorkspace} className="workspace-create">
            <input
              type="text"
              value={newWorkspaceName}
              onChange={(e) => setNewWorkspaceName(e.target.value)}
              placeholder="Nome do novo workspace…"
              autoFocus
            />
            <button type="submit" className="btn btn-primary btn-sm">＋ Criar</button>
          </form>
          <div className="workspace-divider" />
          <ul className="workspace-list">
            {workspaces.map((ws) => (
              <li
                key={ws.id}
                className={`workspace-item ${currentWorkspaceId === ws.id ? "active" : ""}`}
                onClick={() => {
                  if (currentWorkspaceId !== ws.id) {
                    loadWorkspace(ws.id).catch((err) => alert(err));
                  }
                  setIsOpen(false);
                }}
              >
                <span className="workspace-label">
                  <span className="workspace-name">{ws.name}</span>
                  <span className="workspace-meta">
                    {ws.agent_count} agents · {ws.edge_count} conexões
                  </span>
                </span>
                <ManageButton ws={ws} />
              </li>
            ))}
            {workspaces.length === 0 && (
              <li className="workspace-empty">Nenhum workspace ainda.</li>
            )}
          </ul>
        </div>
      )}
    </div>
  );
}

function App() {
  const state = useWorkspaceStore((s) => s.state);
  const saveNow = useWorkspaceStore((s) => s.saveNow);
  const [showCreate, setShowCreate] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [showActivity, setShowActivity] = useState(false);
  const [showCapabilities, setShowCapabilities] = useState(false);

  useEffect(() => {
    const onNew = () => setShowCreate(true);
    const onSettings = () => setShowSettings(true);
    const onActivity = () => setShowActivity((v) => !v);
    window.addEventListener("maestri:new-agent", onNew);
    window.addEventListener("maestri:open-settings", onSettings);
    window.addEventListener("maestri:toggle-activity", onActivity);
    return () => {
      window.removeEventListener("maestri:new-agent", onNew);
      window.removeEventListener("maestri:open-settings", onSettings);
      window.removeEventListener("maestri:toggle-activity", onActivity);
    };
  }, []);

  // Auto-save periódico conforme settings do workspace.
  const autoSave = state?.settings.auto_save ?? false;
  const autoSaveInterval = state?.settings.auto_save_interval ?? 30;
  useEffect(() => {
    if (!autoSave) return;
    const timer = setInterval(() => {
      void saveNow();
    }, Math.max(5, autoSaveInterval) * 1000);
    return () => clearInterval(timer);
  }, [autoSave, autoSaveInterval, saveNow]);

  return (
    <ReactFlowProvider>
      <div className="app">
        <header className="app-header">
          <div className="app-title">MAESTRI 2.0</div>
          <div className="app-meta">
            <WorkspaceSelector />
            <span className="dot">·</span>
            {state && (
              <>
                <span>{state.agents.length} agents</span>
                <span className="dot">·</span>
              </>
            )}
            <button
              className="btn btn-ghost"
              onClick={() =>
                window.dispatchEvent(new KeyboardEvent("keydown", { key: "k", metaKey: true }))
              }
              title="Command Palette (⌘K)"
            >
              ⌘K
            </button>
            <button
              className="btn btn-ghost"
              onClick={() => setShowActivity((v) => !v)}
              title="Activity"
            >
              ☰
            </button>
            <button
              className="btn btn-ghost"
              onClick={() => setShowCapabilities((v) => !v)}
              title="Capabilities (Workflow / Skills / MCP)"
            >
              ⚡
            </button>
            <button
              className="btn btn-ghost"
              onClick={() => setShowSettings(true)}
              title="Workspace Settings"
            >
              ⚙
            </button>
            <button className="btn btn-ghost" onClick={() => void saveNow()} title="Save (⌘S)">
              Save
            </button>
            <button className="btn btn-primary" onClick={() => setShowCreate(true)}>
              + Agent
            </button>
          </div>
        </header>
        <main className="app-main" style={{ display: "flex" }}>
          <div style={{ flex: 1, minWidth: 0 }}>
            <Canvas />
          </div>
          {showActivity && <ActivityPanel onClose={() => setShowActivity(false)} />}
          {showCapabilities && (
            <CapabilitiesPanel onClose={() => setShowCapabilities(false)} />
          )}
        </main>
        <StatusBar />
        <Toasts />
        {showCreate && <CreateAgentModal onClose={() => setShowCreate(false)} />}
        {showSettings && <SettingsPanel onClose={() => setShowSettings(false)} />}
      </div>
    </ReactFlowProvider>
  );
}

export default App;
