import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  applyNodeChanges,
  useReactFlow,
  useNodesInitialized,
  type Node,
  type Edge,
  type NodeChange,
  type EdgeChange,
  type Connection,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { useWorkspaceStore } from "../../store/workspaceStore";
import { AgentNode } from "./AgentNode";
import { AgentDetailPanel } from "../AgentDetailPanel";
import { CommandPalette } from "../CommandPalette";
import type { Agent } from "../../types/models";

const nodeTypes = { agent: AgentNode };

const DEFAULT_W = 520;
const DEFAULT_H = 360;

interface CtxMenu {
  x: number;
  y: number;
  kind: "node" | "pane" | "edge";
  id?: string;
}

function buildNode(
  agent: Agent,
  selectedIds: Set<string>,
  dimmedIds: Set<string> | null,
  onOpenConfig: (id: string) => void,
  prev?: Node
): Node {
  const dimmed = dimmedIds !== null && !dimmedIds.has(agent.id);
  return {
    // Preserva campos internos do node anterior (measured, internals, etc.)
    // para o React Flow não perder a referência durante o drag (#015).
    ...(prev ?? {}),
    id: agent.id,
    type: "agent",
    position: prev
      ? prev.position
      : { x: agent.x * 800, y: agent.y * 500 },
    width: agent.width ?? DEFAULT_W,
    height: agent.collapsed ? undefined : agent.height ?? DEFAULT_H,
    data: { agent, onOpenConfig },
    selected: selectedIds.has(agent.id),
    draggable: !agent.locked,
    style: dimmed
      ? { opacity: 0.25, transition: "opacity 150ms ease-out" }
      : { opacity: 1, transition: "opacity 150ms ease-out" },
  };
}

/** Layout em grid: organiza os agents em colunas a partir da origem. */
function gridLayout(agents: Agent[]): Map<string, { x: number; y: number }> {
  const cols = Math.max(1, Math.ceil(Math.sqrt(agents.length)));
  const gapX = DEFAULT_W + 60;
  const gapY = DEFAULT_H + 60;
  const positions = new Map<string, { x: number; y: number }>();
  agents.forEach((agent, i) => {
    const col = i % cols;
    const row = Math.floor(i / cols);
    positions.set(agent.id, { x: col * gapX, y: row * gapY });
  });
  return positions;
}

export function Canvas() {
  const {
    state,
    loading,
    error,
    load,
    currentWorkspaceId,
    updatePosition,
    updatePositionLocal,
    updateGeometry,
    updateGeometryLocal,
    addEdge,
    removeEdge,
    deleteAgent,
    duplicateAgent,
    startAgent,
    stopAgent,
    restartAgent,
    undo,
    redo,
    saveNow,
  } = useWorkspaceStore();

  const [nodes, setNodes] = useState<Node[]>([]);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [configId, setConfigId] = useState<string | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [ctxMenu, setCtxMenu] = useState<CtxMenu | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchOpen, setSearchOpen] = useState(false);

  // IDs que correspondem à busca atual (null = busca inativa).
  const searchMatchIds = useMemo<Set<string> | null>(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q || !state) return null;
    return new Set(
      state.agents
        .filter(
          (a) =>
            a.name.toLowerCase().includes(q) ||
            a.role.toLowerCase().includes(q) ||
            a.runtime.toLowerCase().includes(q) ||
            a.model.toLowerCase().includes(q)
        )
        .map((a) => a.id)
    );
  }, [searchQuery, state]);

  // ID "principal" da seleção (último clicado) — usado por ações singulares.
  const selectedId = useMemo(() => {
    if (selectedIds.size === 0) return null;
    return [...selectedIds][selectedIds.size - 1];
  }, [selectedIds]);
  void selectedId; // reservado para ações singulares futuras

  const debounceRef = useRef<Record<string, ReturnType<typeof setTimeout>>>(
    {}
  );
  const flowWrapper = useRef<HTMLDivElement>(null);
  const rf = useReactFlow();
  const didInitialFit = useRef(false);

  useEffect(() => {
    load();
  }, [load]);

  // Ao trocar de workspace, limpa seleção/menus e re-enquadra o canvas.
  const prevWorkspaceId = useRef<string | null>(null);
  useEffect(() => {
    if (prevWorkspaceId.current !== null && prevWorkspaceId.current !== currentWorkspaceId) {
      setSelectedIds(new Set());
      setConfigId(null);
      setCtxMenu(null);
      setSearchQuery("");
      setSearchOpen(false);
      // Re-enquadra o novo workspace: o efeito de useNodesInitialized
      // abaixo roda fitView assim que os novos nodes forem medidos.
      didInitialFit.current = false;
    }
    prevWorkspaceId.current = currentWorkspaceId;
  }, [currentWorkspaceId]);

  const openConfig = useCallback((id: string) => {
    setConfigId(id);
  }, []);

  // Sincroniza nodes locais quando os agentes do backend mudam.
  useEffect(() => {
    if (!state) return;
    setNodes((prev) => {
      const prevById = new Map(prev.map((n) => [n.id, n]));
      return state.agents.map((agent) =>
        buildNode(agent, selectedIds, searchMatchIds, openConfig, prevById.get(agent.id))
      );
    });
  }, [state, selectedIds, searchMatchIds, openConfig]);

  // fitView após o PRIMEIRO load com nodes — o fitView do ReactFlow só
  // roda no init (quando ainda não há nodes), então re-enquadramos aqui
  // assim que os nodes terminarem de ser medidos.
  const nodesInitialized = useNodesInitialized();
  useEffect(() => {
    if (!didInitialFit.current && nodesInitialized && nodes.length > 0) {
      didInitialFit.current = true;
      rf.fitView({ padding: 0.15, duration: 300 });
    }
  }, [nodesInitialized, nodes.length, rf]);

  const edges: Edge[] = useMemo(() => {
    if (!state) return [];
    return state.edges.map((e) => ({
      id: e.id,
      source: e.source,
      target: e.target,
      sourceHandle: e.source_handle ?? undefined,
      targetHandle: e.target_handle ?? undefined,
      animated: true,
      style: { stroke: "#475569", strokeWidth: 1.5 },
    }));
  }, [state]);

  const configAgent = useMemo(
    () => state?.agents.find((a) => a.id === configId) ?? null,
    [state, configId]
  );

  const onNodesChange = useCallback(
    (changes: NodeChange[]) => {
      setNodes((nds) => applyNodeChanges(changes, nds));

      for (const change of changes) {
        if (change.type === "position" && change.position) {
          const id = change.id;
          const { x, y } = change.position;
          updatePositionLocal(id, x / 800, y / 500);
          if (debounceRef.current[id]) {
            clearTimeout(debounceRef.current[id]);
          }
          debounceRef.current[id] = setTimeout(() => {
            updatePosition(id, x / 800, y / 500);
          }, 400);
        }
        if (change.type === "dimensions" && change.dimensions) {
          const id = change.id;
          const { width, height } = change.dimensions;
          // Ignora a medição inicial: só persiste quando o tamanho
          // realmente muda em relação ao que está no store.
          const agent = state?.agents.find((a) => a.id === id);
          const storedW = agent?.width ?? DEFAULT_W;
          const storedH = agent?.height ?? DEFAULT_H;
          if (
            Math.abs(width - storedW) < 2 &&
            Math.abs(height - storedH) < 2
          ) {
            continue;
          }
          const node = nodes.find((n) => n.id === id);
          if (node) {
            const { x, y } = node.position;
            updateGeometryLocal(id, x / 800, y / 500, width, height);
            if (debounceRef.current[`geo-${id}`]) {
              clearTimeout(debounceRef.current[`geo-${id}`]);
            }
            debounceRef.current[`geo-${id}`] = setTimeout(() => {
              updateGeometry(id, x / 800, y / 500, width, height);
            }, 400);
          }
        }
        if (change.type === "select") {
          setSelectedIds((prev) => {
            const next = new Set(prev);
            if (change.selected) next.add(change.id);
            else next.delete(change.id);
            return next;
          });
        }
        if (change.type === "remove") {
          void deleteAgent(change.id).catch(() => {});
        }
      }
    },
    [updatePosition, updatePositionLocal, updateGeometry, updateGeometryLocal, deleteAgent, nodes, state]
  );

  // --- Bulk operations (multi-select) ---
  const bulkDelete = useCallback(async () => {
    const ids = [...selectedIds];
    if (ids.length === 0) return;
    await useWorkspaceStore.getState().deleteAgents(ids);
    setSelectedIds(new Set());
  }, [selectedIds]);

  const bulkDuplicate = useCallback(async () => {
    const ids = [...selectedIds];
    for (const id of ids) {
      await duplicateAgent(id).catch(() => {});
    }
  }, [selectedIds, duplicateAgent]);

  const bulkStart = useCallback(async () => {
    for (const id of selectedIds) {
      await startAgent(id).catch(() => {});
    }
  }, [selectedIds, startAgent]);

  const bulkStop = useCallback(async () => {
    for (const id of selectedIds) {
      await stopAgent(id).catch(() => {});
    }
  }, [selectedIds, stopAgent]);

  const bulkRestart = useCallback(async () => {
    for (const id of selectedIds) {
      await restartAgent(id).catch(() => {});
    }
  }, [selectedIds, restartAgent]);

  // --- Auto layout (grid) ---
  const autoLayout = useCallback(async () => {
    if (!state || state.agents.length === 0) return;
    const positions = gridLayout(state.agents);
    const entries = state.agents
      .map((agent) => {
        const pos = positions.get(agent.id);
        return pos ? { id: agent.id, x: pos.x / 800, y: pos.y / 500 } : null;
      })
      .filter((e): e is { id: string; x: number; y: number } => e !== null);
    await useWorkspaceStore.getState().applyLayout(entries);
    useWorkspaceStore.getState().pushToast("Layout organizado em grid", "success");
    requestAnimationFrame(() => rf.fitView({ padding: 0.15, duration: 300 }));
  }, [state, rf]);

  const onEdgesChange = useCallback(
    (changes: EdgeChange[]) => {
      for (const change of changes) {
        if (change.type === "remove") {
          void removeEdge(change.id).catch(() => {});
        }
      }
    },
    [removeEdge]
  );

  const onConnect = useCallback(
    (connection: Connection) => {
      if (!connection.source || !connection.target) return;
      void addEdge({
        source: connection.source,
        target: connection.target,
        source_handle: connection.sourceHandle ?? null,
        target_handle: connection.targetHandle ?? null,
      }).catch((e) => alert(String(e)));
    },
    [addEdge]
  );

  const onPaneClick = useCallback(() => {
    setSelectedIds(new Set());
    setCtxMenu(null);
  }, []);

  // --- Context menus ---
  const onNodeContextMenu = useCallback((e: React.MouseEvent, node: Node) => {
    e.preventDefault();
    setCtxMenu({ x: e.clientX, y: e.clientY, kind: "node", id: node.id });
  }, []);

  const onPaneContextMenu = useCallback((e: React.MouseEvent | MouseEvent) => {
    e.preventDefault();
    setCtxMenu({ x: e.clientX, y: e.clientY, kind: "pane" });
  }, []);

  const onEdgeContextMenu = useCallback((e: React.MouseEvent, edge: Edge) => {
    e.preventDefault();
    setCtxMenu({ x: e.clientX, y: e.clientY, kind: "edge", id: edge.id });
  }, []);

  // --- Keyboard shortcuts ---
  useEffect(() => {
    const isTerminalFocused = () => {
      const el = document.activeElement;
      return !!el && !!el.closest(".xterm");
    };
    const isInputFocused = () => {
      const el = document.activeElement;
      return (
        !!el &&
        (el.tagName === "INPUT" ||
          el.tagName === "TEXTAREA" ||
          el.tagName === "SELECT")
      );
    };

    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;

      // Cmd/Ctrl+K — command palette (sempre)
      if (mod && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((o) => !o);
        return;
      }
      // Cmd/Ctrl+S — save
      if (mod && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void saveNow();
        return;
      }
      // Cmd/Ctrl+Z / Cmd/Ctrl+Shift+Z — undo/redo
      if (mod && e.key.toLowerCase() === "z" && !isTerminalFocused()) {
        e.preventDefault();
        if (e.shiftKey) void redo();
        else void undo();
        return;
      }
      // Cmd/Ctrl+D — duplicate selected (todos os selecionados)
      if (mod && e.key.toLowerCase() === "d" && selectedIds.size > 0 && !isTerminalFocused()) {
        e.preventDefault();
        void bulkDuplicate();
        return;
      }
      // Cmd/Ctrl+A — select all agents
      if (mod && e.key.toLowerCase() === "a" && !isTerminalFocused() && !isInputFocused()) {
        e.preventDefault();
        if (state) setSelectedIds(new Set(state.agents.map((a) => a.id)));
        return;
      }
      // Cmd/Ctrl+F — busca no canvas (quando o terminal NÃO está focado)
      if (mod && e.key.toLowerCase() === "f" && !isTerminalFocused()) {
        e.preventDefault();
        setSearchOpen(true);
        return;
      }
      // Cmd/Ctrl+L — auto layout
      if (mod && e.key.toLowerCase() === "l" && !isTerminalFocused() && !isInputFocused()) {
        e.preventDefault();
        void autoLayout();
        return;
      }
      // F — fit view
      if (e.key.toLowerCase() === "f" && !mod && !isTerminalFocused() && !isInputFocused()) {
        e.preventDefault();
        rf.fitView({ padding: 0.2, duration: 300 });
        return;
      }
      // Escape — fecha painéis/menus/busca
      if (e.key === "Escape") {
        setCtxMenu(null);
        setPaletteOpen(false);
        setConfigId(null);
        if (searchOpen) {
          setSearchOpen(false);
          setSearchQuery("");
        }
      }
    };

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selectedIds, undo, redo, saveNow, bulkDuplicate, autoLayout, rf, state, searchOpen]);

  // Eventos disparados pela Command Palette.
  useEffect(() => {
    const onAutoLayout = () => void autoLayout();
    const onSelectAll = () => {
      if (state) setSelectedIds(new Set(state.agents.map((a) => a.id)));
    };
    window.addEventListener("maestri:auto-layout", onAutoLayout);
    window.addEventListener("maestri:select-all", onSelectAll);
    return () => {
      window.removeEventListener("maestri:auto-layout", onAutoLayout);
      window.removeEventListener("maestri:select-all", onSelectAll);
    };
  }, [autoLayout, state]);

  if (loading) {
    return (
      <div className="canvas-message">Loading workspace…</div>
    );
  }

  if (error) {
    return (
      <div className="canvas-message" style={{ color: "#f87171" }}>
        Unable to load workspace.{" "}
        <button className="btn btn-primary" onClick={() => load()}>
          Retry
        </button>
      </div>
    );
  }

  const ctxAgent = ctxMenu?.id
    ? state?.agents.find((a) => a.id === ctxMenu.id)
    : null;

  const menuItem: React.CSSProperties = {
    display: "block",
    width: "100%",
    textAlign: "left",
    background: "transparent",
    border: "none",
    color: "#cbd5e1",
    fontSize: 12,
    padding: "6px 12px",
    borderRadius: 5,
    cursor: "pointer",
  };

  return (
    <div
      ref={flowWrapper}
      style={{ width: "100%", height: "100%", display: "flex" }}
      onClick={() => setCtxMenu(null)}
    >
      <div style={{ flex: 1, minWidth: 0, position: "relative" }}>
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          onConnect={onConnect}
          onPaneClick={onPaneClick}
          onNodeContextMenu={onNodeContextMenu}
          onPaneContextMenu={onPaneContextMenu}
          onEdgeContextMenu={onEdgeContextMenu}
          fitView
          multiSelectionKeyCode="Shift"
          selectionOnDrag={false}
          snapToGrid={state?.settings.snap_to_grid ?? false}
          snapGrid={[state?.settings.grid_size ?? 24, state?.settings.grid_size ?? 24]}
          deleteKeyCode={["Backspace", "Delete"]}
          proOptions={{ hideAttribution: true }}
        >
          {(state?.settings.show_grid ?? true) && (
            <Background color="#1e293b" gap={state?.settings.grid_size ?? 24} />
          )}
          <Controls />
          {(state?.settings.show_minimap ?? true) && (
            <MiniMap
              nodeColor="#1e293b"
              maskColor="rgba(2, 6, 23, 0.7)"
              style={{ background: "#0f172a" }}
            />
          )}
        </ReactFlow>

        {/* Barra de busca do canvas (Cmd/Ctrl+F) */}
        {searchOpen && (
          <div
            style={{
              position: "absolute",
              top: 12,
              left: "50%",
              transform: "translateX(-50%)",
              display: "flex",
              alignItems: "center",
              gap: 8,
              background: "var(--color-bg-modal)",
              border: "1px solid var(--color-border-default)",
              borderRadius: "var(--radius-lg)",
              padding: "6px 10px",
              boxShadow: "var(--shadow-lg)",
              zIndex: 50,
              minWidth: 320,
            }}
          >
            <input
              autoFocus
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  setSearchOpen(false);
                  setSearchQuery("");
                }
                if (e.key === "Enter" && searchMatchIds && searchMatchIds.size > 0) {
                  const first = [...searchMatchIds][0];
                  const agent = state?.agents.find((a) => a.id === first);
                  if (agent) {
                    rf.setCenter(agent.x * 800 + 260, agent.y * 500 + 180, {
                      zoom: 1,
                      duration: 300,
                    });
                    setSelectedIds(new Set([first]));
                  }
                }
              }}
              placeholder="Buscar agents… (nome, role, runtime)"
              style={{
                flex: 1,
                background: "transparent",
                border: "none",
                outline: "none",
                color: "var(--color-text-primary)",
                fontSize: "var(--text-sm)",
              }}
            />
            <span style={{ fontSize: "var(--text-xs)", color: "var(--color-text-subtle)" }}>
              {searchMatchIds ? `${searchMatchIds.size} encontrados` : ""}
            </span>
            <button
              className="icon-btn"
              onClick={() => {
                setSearchOpen(false);
                setSearchQuery("");
              }}
              aria-label="Fechar busca"
            >
              ✕
            </button>
          </div>
        )}

        {/* Barra de ações de multi-seleção */}
        {selectedIds.size > 1 && (
          <div
            style={{
              position: "absolute",
              bottom: 16,
              left: "50%",
              transform: "translateX(-50%)",
              display: "flex",
              alignItems: "center",
              gap: 8,
              background: "var(--color-bg-modal)",
              border: "1px solid var(--color-border-default)",
              borderRadius: "var(--radius-lg)",
              padding: "6px 10px",
              boxShadow: "var(--shadow-lg)",
              zIndex: 50,
            }}
          >
            <span style={{ fontSize: "var(--text-xs)", color: "var(--color-text-muted)" }}>
              {selectedIds.size} selecionados
            </span>
            <button className="btn btn-ghost btn-sm" onClick={() => void bulkStart()}>▶ Start</button>
            <button className="btn btn-ghost btn-sm" onClick={() => void bulkStop()}>■ Stop</button>
            <button className="btn btn-ghost btn-sm" onClick={() => void bulkRestart()}>↻ Restart</button>
            <button className="btn btn-ghost btn-sm" onClick={() => void bulkDuplicate()}>⧉ Duplicate</button>
            <button
              className="btn btn-ghost btn-sm"
              style={{ color: "var(--color-accent-error)" }}
              onClick={() => void bulkDelete()}
            >
              🗑 Delete
            </button>
          </div>
        )}
        {state && state.agents.length === 0 && (
          <div
            className="canvas-message"
            style={{
              position: "absolute",
              inset: 0,
              pointerEvents: "none",
              background: "transparent",
            }}
          >
            <div style={{ fontSize: 20, fontWeight: 700, marginBottom: 8 }}>
              MAESTRI 2.0
            </div>
            <div style={{ color: "#64748b", marginBottom: 16 }}>
              Create your first agent
            </div>
            <div style={{ color: "#475569", fontSize: 12 }}>
              ⌘K para abrir a Command Palette
            </div>
          </div>
        )}
      </div>

      {/* Context menu */}
      {ctxMenu && (
        <div
          style={{
            position: "fixed",
            left: ctxMenu.x,
            top: ctxMenu.y,
            background: "#0f172a",
            border: "1px solid #1e293b",
            borderRadius: 8,
            padding: 4,
            minWidth: 170,
            boxShadow: "0 8px 24px rgba(0,0,0,0.5)",
            zIndex: 100,
          }}
          onClick={(e) => e.stopPropagation()}
        >
          {ctxMenu.kind === "node" && ctxAgent && (
            <>
              {selectedIds.size > 1 && selectedIds.has(ctxAgent.id) ? (
                <>
                  <div style={{ ...menuItem, color: "#64748b", cursor: "default" }}>
                    {selectedIds.size} selecionados
                  </div>
                  <button
                    style={menuItem}
                    onClick={() => {
                      void bulkStart();
                      setCtxMenu(null);
                    }}
                  >
                    ▶ Start All
                  </button>
                  <button
                    style={menuItem}
                    onClick={() => {
                      void bulkStop();
                      setCtxMenu(null);
                    }}
                  >
                    ■ Stop All
                  </button>
                  <button
                    style={menuItem}
                    onClick={() => {
                      void bulkRestart();
                      setCtxMenu(null);
                    }}
                  >
                    ↻ Restart All
                  </button>
                  <button
                    style={menuItem}
                    onClick={() => {
                      void bulkDuplicate();
                      setCtxMenu(null);
                    }}
                  >
                    ⧉ Duplicate All
                  </button>
                  <button
                    style={{ ...menuItem, color: "#fca5a5" }}
                    onClick={() => {
                      void bulkDelete();
                      setCtxMenu(null);
                    }}
                  >
                    🗑 Delete All
                  </button>
                </>
              ) : (
                <>
                  {ctxAgent.status === "running" || ctxAgent.status === "starting" ? (
                    <button
                      style={menuItem}
                      onClick={() => {
                        void stopAgent(ctxAgent.id);
                        setCtxMenu(null);
                      }}
                    >
                      ■ Stop
                    </button>
                  ) : (
                    <button
                      style={menuItem}
                      onClick={() => {
                        void startAgent(ctxAgent.id);
                        setCtxMenu(null);
                      }}
                    >
                      ▶ Start
                    </button>
                  )}
                  <button
                    style={menuItem}
                    onClick={() => {
                      void restartAgent(ctxAgent.id);
                      setCtxMenu(null);
                    }}
                  >
                    ↻ Restart
                  </button>
                  <button
                    style={menuItem}
                    onClick={() => {
                      setConfigId(ctxAgent.id);
                      setCtxMenu(null);
                    }}
                  >
                    ⚙ Configure
                  </button>
                  <button
                    style={menuItem}
                    onClick={() => {
                      void duplicateAgent(ctxAgent.id);
                      setCtxMenu(null);
                    }}
                  >
                    ⧉ Duplicate
                  </button>
                  <button
                    style={{ ...menuItem, color: "#fca5a5" }}
                    onClick={() => {
                      void deleteAgent(ctxAgent.id);
                      setCtxMenu(null);
                    }}
                  >
                    🗑 Delete
                  </button>
                </>
              )}
            </>
          )}
          {ctxMenu.kind === "pane" && (
            <>
              <button
                style={menuItem}
                onClick={() => {
                  setPaletteOpen(true);
                  setCtxMenu(null);
                }}
              >
                + New Agent
              </button>
              <button
                style={menuItem}
                onClick={() => {
                  void autoLayout();
                  setCtxMenu(null);
                }}
              >
                ⊞ Auto Layout
              </button>
              <button
                style={menuItem}
                onClick={() => {
                  if (state) setSelectedIds(new Set(state.agents.map((a) => a.id)));
                  setCtxMenu(null);
                }}
              >
                ☐ Select All
              </button>
              <button
                style={menuItem}
                onClick={() => {
                  rf.fitView({ padding: 0.2, duration: 300 });
                  setCtxMenu(null);
                }}
              >
                ⛶ Fit View
              </button>
            </>
          )}
          {ctxMenu.kind === "edge" && ctxMenu.id && (
            <button
              style={{ ...menuItem, color: "#fca5a5" }}
              onClick={() => {
                void removeEdge(ctxMenu.id!);
                setCtxMenu(null);
              }}
            >
              🗑 Delete Connection
            </button>
          )}
        </div>
      )}

      {configAgent && (
        <AgentDetailPanel
          agent={configAgent}
          onClose={() => setConfigId(null)}
        />
      )}

      {paletteOpen && (
        <CommandPalette
          onClose={() => setPaletteOpen(false)}
          onOpenConfig={(id) => {
            setConfigId(id);
            setPaletteOpen(false);
          }}
          onCenterNode={(id) => {
            const agent = state?.agents.find((a) => a.id === id);
            if (agent) {
              rf.setCenter(agent.x * 800 + 260, agent.y * 500 + 180, {
                zoom: 1,
                duration: 300,
              });
              setSelectedIds(new Set([id]));
            }
            setPaletteOpen(false);
          }}
        />
      )}
    </div>
  );
}
