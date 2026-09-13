import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { routeOutput } from "../terminal/terminalRegistry";
import type {
  Agent,
  CreateAgentInput,
  AskReport,
  DeliveryReport,
  Edge,
  UpdateAgentInput,
  WorkspaceState,
  WorkspaceListItem,
  CreateWorkspaceInput,
  RenameWorkspaceInput,
  UpdateWorkspaceSettingsInput,
  UpdateViewportInput,
  Project,
  CreateProjectInput,
  RenameProjectInput,
  ProjectWorkspaceInput,
  Skill,
  CreateSkillInput,
  McpServer,
  CreateMcpServerInput,
  MemoryEntry,
  CreateMemoryInput,
  ChainExecution,
  WorkflowEvent,
} from "../types/models";

/** Payload de evento emitido pelo backend: `{ agentId, data }`. */
interface AgentEvent {
  agentId: string;
  data: string;
}

export type SaveState = "saved" | "saving" | "unsaved";

export interface Toast {
  id: number;
  message: string;
  kind: "info" | "success" | "error";
}

export interface ActivityEntry {
  id: number;
  ts: number;
  message: string;
  kind: "info" | "success" | "error";
}

const HISTORY_LIMIT = 50;
const ACTIVITY_LIMIT = 200;
let toastSeq = 0;

interface WorkspaceStore {
  state: WorkspaceState | null;
  loading: boolean;
  error: string | null;
  saveState: SaveState;
  toasts: Toast[];
  /** Log de atividade da sessão (eventos, ações, erros). */
  activity: ActivityEntry[];
  /** Histórico estrutural para undo/redo (snapshots de WorkspaceState). */
  past: WorkspaceState[];
  future: WorkspaceState[];
  /** Lista de workspaces disponíveis. */
  workspaces: WorkspaceListItem[];
  /** ID do workspace atualmente carregado. */
  currentWorkspaceId: string | null;
  /** Lista de projetos disponíveis (Fase 12). */
  projects: Project[];

  load: () => Promise<void>;
  createAgent: (input: CreateAgentInput) => Promise<Agent>;
  updateAgent: (input: UpdateAgentInput) => Promise<Agent>;
  deleteAgent: (id: string) => Promise<void>;
  /** Remove vários agents com uma única entrada de histórico. */
  deleteAgents: (ids: string[]) => Promise<void>;
  /** Aplica posições de layout com uma única entrada de histórico. */
  applyLayout: (entries: Array<{ id: string; x: number; y: number }>) => Promise<void>;
  duplicateAgent: (id: string) => Promise<Agent>;
  updatePosition: (id: string, x: number, y: number) => Promise<void>;
  updateGeometry: (
    id: string,
    x: number,
    y: number,
    width: number,
    height: number
  ) => Promise<void>;
  updatePositionLocal: (id: string, x: number, y: number) => void;
  updateGeometryLocal: (
    id: string,
    x: number,
    y: number,
    width: number,
    height: number
  ) => void;
  addEdge: (input: {
    source: string;
    target: string;
    source_handle?: string | null;
    target_handle?: string | null;
  }) => Promise<Edge>;
  removeEdge: (id: string) => Promise<void>;
  startAgent: (id: string) => Promise<void>;
  stopAgent: (id: string) => Promise<void>;
  restartAgent: (id: string) => Promise<void>;
  refreshAgent: (id: string) => Promise<void>;
  sendInput: (id: string, input: string) => Promise<void>;
  resizeAgent: (id: string, cols: number, rows: number) => Promise<void>;
  /** Envia contexto de um agente para os destinos conectados (Fase 4). */
  routeContext: (sourceId: string, payload: string) => Promise<DeliveryReport[]>;
  /** Faz um pedido (ask) de um agente para um peer (request/reply, Fase 4). */
  routeContextAsk: (
    sourceId: string,
    target: string,
    payload: string
  ) => Promise<AskReport>;
  setAgentStatus: (id: string, status: Agent["status"]) => void;

  // Workspace management
  loadWorkspaces: () => Promise<void>;
  createWorkspace: (input: CreateWorkspaceInput) => Promise<WorkspaceState>;
  loadWorkspace: (id: string) => Promise<WorkspaceState>;
  renameWorkspace: (input: RenameWorkspaceInput) => Promise<WorkspaceState>;
  deleteWorkspace: (id: string) => Promise<void>;
  duplicateWorkspace: (id: string) => Promise<WorkspaceState>;
  updateWorkspaceSettings: (input: UpdateWorkspaceSettingsInput) => Promise<WorkspaceState>;
  updateViewport: (input: UpdateViewportInput) => Promise<void>;
  setCurrentWorkspace: (workspace: WorkspaceState) => void;

  // Projects (Fase 12)
  listProjects: () => Promise<Project[]>;
  createProject: (input: CreateProjectInput) => Promise<Project>;
  renameProject: (input: RenameProjectInput) => Promise<Project>;
  deleteProject: (id: string) => Promise<void>;
  addWorkspaceToProject: (input: ProjectWorkspaceInput) => Promise<Project>;
  removeWorkspaceFromProject: (input: ProjectWorkspaceInput) => Promise<Project>;

  undo: () => Promise<void>;
  redo: () => Promise<void>;
  exportWorkspace: () => Promise<string>;
  importWorkspace: (json: string) => Promise<void>;
  saveNow: () => Promise<void>;
  backupWorkspace: () => Promise<string>;
  listBackups: () => Promise<string[]>;

  pushToast: (message: string, kind?: Toast["kind"]) => void;
  dismissToast: (id: number) => void;
  logActivity: (message: string, kind?: ActivityEntry["kind"]) => void;
  clearActivity: () => void;

  // Skills / MCP / Workflow (Fases 6–10, integração com a UI)
  runChain: (payload: string) => Promise<ChainExecution>;
  listWorkflowEvents: () => Promise<WorkflowEvent[]>;
  listSkills: () => Promise<Skill[]>;
  createSkill: (input: CreateSkillInput) => Promise<Skill>;
  associateSkill: (agentId: string, skillId: string) => Promise<void>;
  deleteSkill: (id: string) => Promise<void>;
  listMcpServers: () => Promise<McpServer[]>;
  createMcpServer: (input: CreateMcpServerInput) => Promise<McpServer>;
  associateMcpToAgent: (agentId: string, serverId: string) => Promise<void>;
  deleteMcpServer: (id: string) => Promise<void>;
  // Memory (Fase 11)
  listMemory: () => Promise<MemoryEntry[]>;
  createMemory: (input: CreateMemoryInput) => Promise<MemoryEntry>;
  removeMemory: (id: string) => Promise<void>;
}

export const useWorkspaceStore = create<WorkspaceStore>((set, get) => {
  const markUnsaved = () => set({ saveState: "unsaved" });

  /** Empilha snapshot atual no histórico antes de uma mutação estrutural. */
  const pushHistory = () => {
    const { state, past } = get();
    if (!state) return;
    const next = [...past, state];
    if (next.length > HISTORY_LIMIT) next.shift();
    set({ past: next, future: [] });
  };

  return {
    state: null,
    loading: false,
    error: null,
    saveState: "saved",
    toasts: [],
    activity: [],
    past: [],
    future: [],
    workspaces: [],
    currentWorkspaceId: null,
    projects: [],

    load: async () => {
      set({ loading: true, error: null });
      try {
        const state = await invoke<WorkspaceState>("get_workspace_state");
        set({ state, loading: false, saveState: "saved", currentWorkspaceId: state.metadata.id });

        // Carrega a lista de workspaces para o seletor
        get().loadWorkspaces();
        get().listProjects().catch(() => {});

        // Auto-start agents configurados com auto_start: true
        if (state) {
          for (const agent of state.agents) {
            if (agent.auto_start && agent.status !== "running" && agent.status !== "starting") {
              // Usa setTimeout para não bloquear o load
              setTimeout(() => {
                get().refreshAgent(agent.id).catch(() => {});
              }, 100);
            }
          }
        }
      } catch (e) {
        set({ error: String(e), loading: false });
      }
    },

    // Workspace management
    loadWorkspaces: async () => {
      try {
        const workspaces = await invoke<WorkspaceListItem[]>("list_workspaces");
        set({ workspaces });
      } catch (e) {
        get().pushToast(`Falha ao carregar workspaces: ${e}`, "error");
      }
    },

    createWorkspace: async (input) => {
      const workspace = await invoke<WorkspaceState>("create_workspace", { input });
      const listItem: WorkspaceListItem = {
        id: workspace.metadata.id,
        name: workspace.metadata.name,
        agent_count: workspace.agents.length,
        edge_count: workspace.edges.length,
        updated_at: workspace.metadata.updated_at,
        last_opened_at: workspace.metadata.last_opened_at,
      };
      set((state) => ({
        workspaces: [listItem, ...state.workspaces],
        state: workspace,
        currentWorkspaceId: workspace.metadata.id,
        saveState: "saved",
      }));
      get().pushToast(`Workspace "${workspace.metadata.name}" criado`, "success");
      return workspace;
    },

    loadWorkspace: async (id) => {
      const workspace = await invoke<WorkspaceState>("load_workspace", { id });
      set({
        state: workspace,
        currentWorkspaceId: workspace.metadata.id,
        saveState: "saved",
        // Limpa histórico de undo/redo — snapshots são do workspace anterior.
        past: [],
        future: [],
      });
      get().pushToast(`Workspace "${workspace.metadata.name}" carregado`, "success");

      // Auto-start agents configurados (mesmo comportamento do load inicial).
      for (const agent of workspace.agents) {
        if (agent.auto_start && agent.status !== "running" && agent.status !== "starting") {
          setTimeout(() => {
            get().refreshAgent(agent.id).catch(() => {});
          }, 100);
        }
      }
      return workspace;
    },

    renameWorkspace: async (input: RenameWorkspaceInput) => {
      const workspace = await invoke<WorkspaceState>("rename_workspace", { input });
      const listItem: WorkspaceListItem = {
        id: workspace.metadata.id,
        name: workspace.metadata.name,
        agent_count: workspace.agents.length,
        edge_count: workspace.edges.length,
        updated_at: workspace.metadata.updated_at,
        last_opened_at: workspace.metadata.last_opened_at,
      };
      set((state) => {
        const newWorkspaces = state.workspaces.map((w) => (w.id === input.id ? listItem : w));
        return { workspaces: newWorkspaces };
      });
      get().pushToast(`Workspace renomeado para "${workspace.metadata.name}"`, "success");
      return workspace;
    },

    deleteWorkspace: async (id: string) => {
      const wasCurrent = get().currentWorkspaceId === id;
      await invoke<void>("delete_workspace", { id });
      set((state) => {
        const newWorkspaces = state.workspaces.filter((w) => w.id !== id);
        return { workspaces: newWorkspaces };
      });
      // Se deletou o workspace atual, o backend já trocou para outro:
      // re-sincroniza o estado com o workspace corrente do backend.
      if (wasCurrent) {
        const current = await invoke<WorkspaceState>("get_workspace_state");
        set({
          state: current,
          currentWorkspaceId: current.metadata.id,
          saveState: "saved",
        });
      }
      get().pushToast("Workspace deletado", "info");
    },

    duplicateWorkspace: async (id: string) => {
      const workspace = await invoke<WorkspaceState>("duplicate_workspace", { id });
      const listItem: WorkspaceListItem = {
        id: workspace.metadata.id,
        name: workspace.metadata.name,
        agent_count: workspace.agents.length,
        edge_count: workspace.edges.length,
        updated_at: workspace.metadata.updated_at,
        last_opened_at: workspace.metadata.last_opened_at,
      };
      set((state) => {
        const newWorkspaces = [listItem, ...state.workspaces];
        return { workspaces: newWorkspaces };
      });
      get().pushToast(`Workspace duplicado: "${workspace.metadata.name}"`, "success");
      return workspace;
    },

    updateWorkspaceSettings: async (input: UpdateWorkspaceSettingsInput) => {
      const workspace = await invoke<WorkspaceState>("update_workspace_settings", { input });
      const listItem: WorkspaceListItem = {
        id: workspace.metadata.id,
        name: workspace.metadata.name,
        agent_count: workspace.agents.length,
        edge_count: workspace.edges.length,
        updated_at: workspace.metadata.updated_at,
        last_opened_at: workspace.metadata.last_opened_at,
      };
      set((state) => {
        const newWorkspaces = state.workspaces.map((w) => (w.id === input.id ? listItem : w));
        // BUG FIX: também atualiza `state` para o Canvas aplicar as novas
        // settings (grid/snap/minimap) imediatamente, sem reload.
        return { workspaces: newWorkspaces, state: workspace };
      });
      get().pushToast("Configurações do workspace atualizadas", "success");
      return workspace;
    },

    updateViewport: async (input) => {
      await invoke<void>("update_viewport", { input });
    },

    setCurrentWorkspace: (workspace) => {
      set({ state: workspace, currentWorkspaceId: workspace.metadata.id });
    },

    // Projects (Fase 12)
    listProjects: async () => {
      const projects = await invoke<Project[]>("list_projects");
      set({ projects });
      return projects;
    },

    createProject: async (input) => {
      const project = await invoke<Project>("create_project", { input });
      set((s) => ({ projects: [...s.projects, project] }));
      get().pushToast(`Projeto "${project.name}" criado`, "success");
      return project;
    },

    renameProject: async (input) => {
      const project = await invoke<Project>("rename_project", { input });
      set((s) => ({
        projects: s.projects.map((p) => (p.id === input.id ? project : p)),
      }));
      get().pushToast(`Projeto renomeado para "${project.name}"`, "success");
      return project;
    },

    deleteProject: async (id) => {
      await invoke<void>("delete_project", { id });
      set((s) => ({ projects: s.projects.filter((p) => p.id !== id) }));
      get().pushToast("Projeto deletado", "info");
    },

    addWorkspaceToProject: async (input) => {
      const project = await invoke<Project>("add_workspace_to_project", { input });
      set((s) => ({
        projects: s.projects.map((p) => (p.id === input.project_id ? project : p)),
      }));
      get().pushToast(`Workspace associado ao projeto "${project.name}"`, "success");
      return project;
    },

    removeWorkspaceFromProject: async (input) => {
      const project = await invoke<Project>("remove_workspace_from_project", { input });
      set((s) => ({
        projects: s.projects.map((p) => (p.id === input.project_id ? project : p)),
      }));
      get().pushToast(`Workspace removido do projeto "${project.name}"`, "info");
      return project;
    },

    createAgent: async (input) => {
      pushHistory();
      const agent = await invoke<Agent>("create_agent", { input });
      const state = get().state;
      if (state) {
        set({ state: { ...state, agents: [...state.agents, agent] } });
      } else {
        // Workspace ainda não carregado no store (load pendente/falhou):
        // busca o estado real (já contém o agente persistido) para exibir
        // imediatamente, mantendo a persistência atual do backend.
        const fresh = await invoke<WorkspaceState>("get_workspace_state");
        set({
          state: fresh,
          currentWorkspaceId: fresh.metadata.id,
          saveState: "saved",
        });
      }
      get().pushToast(`Agent "${agent.name}" criado`, "success");
      return agent;
    },

    updateAgent: async (input) => {
      pushHistory();
      const agent = await invoke<Agent>("update_agent", { input });
      const state = get().state;
      if (state) {
        set({
          state: {
            ...state,
            agents: state.agents.map((a) => (a.id === agent.id ? agent : a)),
          },
        });
      }
      return agent;
    },

    deleteAgent: async (id) => {
      pushHistory();
      await invoke<void>("delete_agent", { id });
      const state = get().state;
      if (state) {
        set({
          state: {
            ...state,
            agents: state.agents.filter((a) => a.id !== id),
            edges: state.edges.filter(
              (e) => e.source !== id && e.target !== id
            ),
          },
        });
      }
      get().pushToast("Agent removido", "info");
    },

    duplicateAgent: async (id) => {
      pushHistory();
      const agent = await invoke<Agent>("duplicate_agent", { id });
      const state = get().state;
      if (state) {
        set({ state: { ...state, agents: [...state.agents, agent] } });
      } else {
        const fresh = await invoke<WorkspaceState>("get_workspace_state");
        set({
          state: fresh,
          currentWorkspaceId: fresh.metadata.id,
          saveState: "saved",
        });
      }
      get().pushToast(`Agent duplicado: "${agent.name}"`, "success");
      return agent;
    },

    deleteAgents: async (ids) => {
      if (ids.length === 0) return;
      // Uma única entrada de histórico para toda a operação em lote.
      pushHistory();
      const idSet = new Set(ids);
      for (const id of ids) {
        await invoke<void>("delete_agent", { id }).catch(() => {});
      }
      const state = get().state;
      if (state) {
        set({
          state: {
            ...state,
            agents: state.agents.filter((a) => !idSet.has(a.id)),
            edges: state.edges.filter(
              (e) => !idSet.has(e.source) && !idSet.has(e.target)
            ),
          },
        });
      }
      get().pushToast(`${ids.length} agent(s) removidos`, "info");
    },

    applyLayout: async (entries) => {
      if (entries.length === 0) return;
      // Uma única entrada de histórico para todo o rearranjo.
      pushHistory();
      const posById = new Map(entries.map((e) => [e.id, e]));
      const state = get().state;
      if (state) {
        set({
          state: {
            ...state,
            agents: state.agents.map((a) => {
              const p = posById.get(a.id);
              return p ? { ...a, x: p.x, y: p.y } : a;
            }),
          },
        });
      }
      for (const e of entries) {
        await invoke<void>("update_agent_position", { id: e.id, x: e.x, y: e.y }).catch(() => {});
      }
      set({ saveState: "saved" });
    },

    updatePosition: async (id, x, y) => {
      await invoke<void>("update_agent_position", { id, x, y });
      const state = get().state;
      if (state) {
        set({
          state: {
            ...state,
            agents: state.agents.map((a) =>
              a.id === id ? { ...a, x, y } : a
            ),
          },
        });
      }
    },

    updateGeometry: async (id, x, y, width, height) => {
      await invoke<void>("update_agent_geometry", { id, x, y, width, height });
      const state = get().state;
      if (state) {
        set({
          state: {
            ...state,
            agents: state.agents.map((a) =>
              a.id === id ? { ...a, x, y, width, height } : a
            ),
          },
        });
      }
    },

    updatePositionLocal: (id, x, y) => {
      const state = get().state;
      if (!state) return;
      set({
        state: {
          ...state,
          agents: state.agents.map((a) => (a.id === id ? { ...a, x, y } : a)),
        },
      });
      markUnsaved();
    },

    updateGeometryLocal: (id, x, y, width, height) => {
      const state = get().state;
      if (!state) return;
      set({
        state: {
          ...state,
          agents: state.agents.map((a) =>
            a.id === id ? { ...a, x, y, width, height } : a
          ),
        },
      });
      markUnsaved();
    },

    addEdge: async (input) => {
      pushHistory();
      const edge = await invoke<Edge>("add_edge", { input });
      const state = get().state;
      if (state) {
        set({ state: { ...state, edges: [...state.edges, edge] } });
      }
      get().pushToast("Conexão criada", "success");
      return edge;
    },

    removeEdge: async (id) => {
      pushHistory();
      await invoke<void>("remove_edge", { id });
      const state = get().state;
      if (state) {
        set({
          state: { ...state, edges: state.edges.filter((e) => e.id !== id) },
        });
      }
      get().pushToast("Conexão removida", "info");
    },

    startAgent: async (id) => {
      await invoke<void>("start_agent", { id });
      get().pushToast("Agent iniciado", "success");
    },

    stopAgent: async (id) => {
      await invoke<void>("stop_agent", { id });
      get().pushToast("Agent parado", "info");
    },

    restartAgent: async (id) => {
      await invoke<void>("restart_agent", { id });
      get().pushToast("Agent reiniciado", "success");
    },

    refreshAgent: async (id: string) => {
      await invoke<void>("refresh_agent", { id });
      get().pushToast("Agent atualizado", "success");
    },

    sendInput: async (id, input) => {
      await invoke<void>("send_agent_input", { id, input });
    },

    resizeAgent: async (id, cols, rows) => {
      await invoke<void>("resize_agent", { id, cols, rows });
    },

    routeContext: async (sourceId, payload) => {
      const reports = await invoke<DeliveryReport[]>("route_context", {
        sourceId,
        payload,
      });
      if (reports.length === 0) {
        get().pushToast("Nenhuma conexão de saída para este agente", "info");
      } else {
        const delivered = reports.filter((r) => r.delivered).length;
        const failed = reports.filter((r) => !r.delivered).length;
        const msg =
          delivered === reports.length
            ? `Contexto enviado (${delivered} destino(s))`
            : `Contexto enviado: ${delivered} ok, ${failed} falha(s)`;
        get().pushToast(msg, failed === 0 ? "success" : "error");
      }
      return reports;
    },

    routeContextAsk: async (sourceId, target, payload) => {
      const report = await invoke<AskReport>("route_context_ask", {
        sourceId,
        target,
        payload,
      });
      if (report.delivered) {
        get().pushToast(`Pedido enviado para "${report.target}"`, "success");
      } else {
        get().pushToast(`Pedido falhou: ${report.error ?? "erro"}`, "error");
      }
      return report;
    },

    setAgentStatus: (id, status) => {
      const state = get().state;
      if (!state) return;
      set({
        state: {
          ...state,
          agents: state.agents.map((a) => (a.id === id ? { ...a, status } : a)),
        },
      });
    },

    undo: async () => {
      const { past, state, future } = get();
      if (past.length === 0 || !state) return;
      const previous = past[past.length - 1];
      set({
        past: past.slice(0, -1),
        future: [state, ...future],
        state: previous,
      });
      await invoke<void>("restore_workspace", { snapshot: previous });
      get().pushToast("Undo", "info");
    },

    redo: async () => {
      const { past, state, future } = get();
      if (future.length === 0 || !state) return;
      const next = future[0];
      set({
        past: [...past, state],
        future: future.slice(1),
        state: next,
      });
      await invoke<void>("restore_workspace", { snapshot: next });
      get().pushToast("Redo", "info");
    },

    exportWorkspace: async () => {
      return await invoke<string>("export_workspace");
    },

    importWorkspace: async (json) => {
      pushHistory();
      const state = await invoke<WorkspaceState>("import_workspace", { json });
      set({
        state,
        saveState: "saved",
        currentWorkspaceId: state.metadata.id,
      });
      get().pushToast("Workspace importado", "success");
    },

    saveNow: async () => {
      set({ saveState: "saving" });
      try {
        // O backend persiste em cada mutação (state.persist()). Aqui apenas
        // confirmamos que o backend está acessível — NÃO substituímos `state`,
        // pois isso poderia sobrescrever edições locais em voo (debounce de
        // drag/resize) e fazer nodes "pularem" durante o auto-save.
        await invoke<WorkspaceState>("get_workspace_state");
        set({ saveState: "saved" });
      } catch (e) {
        set({ saveState: "unsaved" });
        get().pushToast(`Falha ao salvar: ${e}`, "error");
      }
    },

    backupWorkspace: async () => {
      const path = await invoke<string>("backup_workspace");
      get().pushToast("Backup criado", "success");
      get().logActivity(`Backup salvo: ${path}`, "success");
      return path;
    },

    listBackups: async () => {
      return await invoke<string[]>("list_backups");
    },

    // ------------------------------------------------------------------
    // Skills / MCP / Workflow (Fases 6–10): finas camadas sobre o IPC.
    // ------------------------------------------------------------------
    runChain: async (payload) => {
      const exec = await invoke<ChainExecution>("run_canvas_chain", { payload });
      get().pushToast(
        exec.status === "completed"
          ? "Workflow concluído"
          : exec.status === "failed"
            ? `Workflow falhou: ${exec.error ?? "erro"}`
            : "Workflow: " + exec.status,
        exec.status === "completed" ? "success" : exec.status === "failed" ? "error" : "info"
      );
      return exec;
    },

    listWorkflowEvents: async () => {
      return await invoke<WorkflowEvent[]>("list_workflow_events");
    },

    listSkills: async () => {
      return await invoke<Skill[]>("list_skills");
    },

    createSkill: async (input) => {
      const skill = await invoke<Skill>("create_skill", { input });
      get().pushToast(`Skill "${skill.name}" criada`, "success");
      return skill;
    },

    associateSkill: async (agentId, skillId) => {
      await invoke<void>("associate_skill", { agentId, skillId });
      get().pushToast("Skill associada ao agente", "success");
    },

    deleteSkill: async (id) => {
      await invoke<void>("delete_skill", { id });
      get().pushToast("Skill removida", "info");
    },

    listMcpServers: async () => {
      return await invoke<McpServer[]>("list_mcp_servers");
    },

    createMcpServer: async (input) => {
      const server = await invoke<McpServer>("create_mcp_server", { input });
      get().pushToast(`MCP server "${server.name}" criado`, "success");
      return server;
    },

    associateMcpToAgent: async (agentId, serverId) => {
      await invoke<void>("associate_mcp_to_agent", { agentId, serverId });
      get().pushToast("MCP server associado ao agente", "success");
    },

    deleteMcpServer: async (id) => {
      await invoke<void>("delete_mcp_server", { id });
      get().pushToast("MCP server removido", "info");
    },

    listMemory: async () => {
      return await invoke<MemoryEntry[]>("list_memory");
    },

    createMemory: async (input) => {
      const entry = await invoke<MemoryEntry>("create_memory", { input });
      get().pushToast(
        entry.agent_id ? "Memória criada (agente)" : "Memória compartilhada criada",
        "success"
      );
      return entry;
    },

    removeMemory: async (id) => {
      await invoke<void>("remove_memory", { id });
      get().pushToast("Memória removida", "info");
    },

    pushToast: (message, kind = "info") => {
      const id = ++toastSeq;
      set({ toasts: [...get().toasts, { id, message, kind }] });
      get().logActivity(message, kind);
      setTimeout(() => {
        get().dismissToast(id);
      }, 3500);
    },

    dismissToast: (id) => {
      set({ toasts: get().toasts.filter((t) => t.id !== id) });
    },

    logActivity: (message, kind = "info") => {
      const id = ++toastSeq;
      const entry: ActivityEntry = { id, ts: Date.now(), message, kind };
      const activity = [...get().activity, entry];
      if (activity.length > ACTIVITY_LIMIT) activity.shift();
      set({ activity });
    },

    clearActivity: () => set({ activity: [] }),
  };
});

// ---------------------------------------------------------------------------
// Listeners de eventos do backend (registrados uma única vez no módulo).
// ---------------------------------------------------------------------------
let listenersAttached = false;

export function attachEventListeners(): void {
  if (listenersAttached) return;
  // Fora do WebView do Tauri (ex.: preview no browser) não há IPC — ignora.
  if (!("__TAURI_INTERNALS__" in window)) return;
  listenersAttached = true;

  const store = () => useWorkspaceStore.getState();

  listen<AgentEvent>("agent_output", (event) => {
    routeOutput(event.payload.agentId, event.payload.data);
  });

  const agentName = (id: string) =>
    store().state?.agents.find((a) => a.id === id)?.name ?? id;

  listen<AgentEvent>("agent_started", (event) => {
    store().setAgentStatus(event.payload.agentId, "running");
    store().logActivity(`Agent iniciado: ${agentName(event.payload.agentId)}`, "success");
  });

  listen<AgentEvent>("agent_stopped", (event) => {
    store().setAgentStatus(event.payload.agentId, "stopped");
    store().logActivity(`Agent parado: ${agentName(event.payload.agentId)}`, "info");
  });

  listen<AgentEvent>("agent_status_changed", (event) => {
    const status = event.payload.data as Agent["status"];
    store().setAgentStatus(event.payload.agentId, status);
  });

  listen<AgentEvent>("agent_error", (event) => {
    store().setAgentStatus(event.payload.agentId, "failed");
    routeOutput(event.payload.agentId, `\n[ERRO] ${event.payload.data}\n`);
    store().pushToast(`Agent falhou: ${event.payload.data}`, "error");
  });
}
