// Tipos compartilhados — espelham os modelos Rust em src-tauri/src/models.rs

export type Role =
  | "orchestrator"
  | "specialist"
  | "auditor"
  | "builder"
  | "reviewer"
  | "qa"
  | "observer";

export type Runtime = "kilo" | "claude_code" | "opencode" | "codex" | "custom";

export type AgentKind = "cli" | "web" | "app";

export type Status =
  | "idle"
  | "starting"
  | "running"
  | "waiting"
  | "done"
  | "failed"
  | "stopped";

export type EdgeType = "message" | "data" | "control" | "workflow";

export interface Agent {
  id: string;
  name: string;
  role: Role;
  runtime: Runtime;
  kind: AgentKind;
  model: string;
  command: string;
  args: string[];
  working_dir: string;
  status: Status;
  x: number;
  y: number;
  width?: number | null;
  height?: number | null;
  collapsed: boolean;
  locked: boolean;
  accent?: string | null;
  auto_start: boolean;
}

/** Conexão entre dois agentes (topologia visual do canvas). */
export interface Edge {
  id: string;
  source: string;
  target: string;
  source_handle?: string | null;
  target_handle?: string | null;
  edge_type: EdgeType;
  label?: string | null;
}

/** Resultado da entrega de uma mensagem roteada (route_context). */
export interface DeliveryReport {
  target: string;
  delivered: boolean;
  error?: string | null;
}

/** Resultado de um pedido (ask) a um peer (route_context_ask). */
export interface AskReport {
  target: string;
  delivered: boolean;
  request_id: string;
  error?: string | null;
}

export interface Viewport {
  x: number;
  y: number;
  zoom: number;
}

export interface WorkspaceSettings {
  show_grid: boolean;
  snap_to_grid: boolean;
  grid_size: number;
  show_minimap: boolean;
  auto_save: boolean;
  auto_save_interval: number;
}

export interface WorkspaceMetadata {
  id: string;
  name: string;
  created_at: number;
  updated_at: number;
  last_opened_at: number;
}

export interface WorkspaceState {
  metadata: WorkspaceMetadata;
  agents: Agent[];
  edges: Edge[];
  viewport: Viewport;
  settings: WorkspaceSettings;
  version: string;
}

export interface WorkspaceListItem {
  id: string;
  name: string;
  agent_count: number;
  edge_count: number;
  updated_at: number;
  last_opened_at: number;
}

/** Runtime detectável localmente (retornado por detect_runtimes). */
export interface RuntimeInfo {
  id: string;
  name: string;
  command: string;
  available: boolean;
  capabilities: string[];
}

/** Payload de criação (sem id — o backend gera). */
export interface CreateAgentInput {
  name: string;
  role: Role;
  runtime: Runtime;
  kind?: AgentKind;
  model: string;
  command: string;
  args: string[];
  working_dir: string;
  auto_start?: boolean;
}

/** Payload de atualização (id obrigatório). */
export interface UpdateAgentInput extends CreateAgentInput {
  id: string;
  collapsed?: boolean;
  locked?: boolean;
  accent?: string | null;
  auto_start?: boolean;
}

/** Payload para criar um novo workspace. */
export interface CreateWorkspaceInput {
  name: string;
}

/** Payload para renomear workspace. */
export interface RenameWorkspaceInput {
  id: string;
  name: string;
}

/** Payload para atualizar configurações do workspace. */
export interface UpdateWorkspaceSettingsInput {
  id: string;
  settings: WorkspaceSettings;
}

/** Payload para atualizar viewport. */
export interface UpdateViewportInput {
  id: string;
  viewport: Viewport;
}

// ---------------------------------------------------------------------------
// Projects (Fase 12)
// ---------------------------------------------------------------------------

/** Um Projeto agrupa múltiplos workspaces (camada organizacional). */
export interface Project {
  id: string;
  name: string;
  workspace_ids: string[];
  created_at: number;
  updated_at: number;
}

/** Payload para criar um projeto. */
export interface CreateProjectInput {
  name: string;
}

/** Payload para renomear um projeto. */
export interface RenameProjectInput {
  id: string;
  name: string;
}

/** Payload para associar/desassociar um workspace a um projeto. */
export interface ProjectWorkspaceInput {
  project_id: string;
  workspace_id: string;
}

// ---------------------------------------------------------------------------
// Skills (Fase 9)
// ---------------------------------------------------------------------------

export interface Skill {
  id: string;
  name: string;
  description: string;
  version?: string | null;
}

export interface CreateSkillInput {
  name: string;
  description: string;
  version?: string | null;
  agent_id?: string | null;
}

// ---------------------------------------------------------------------------
// MCP Manager (Fase 10)
// ---------------------------------------------------------------------------

export type McpTransport = "stdio" | "sse" | "http";

export interface McpServerConfig {
  command?: string | null;
  args?: string[] | null;
  url?: string | null;
  env?: Record<string, string>;
}

export interface McpServer {
  id: string;
  name: string;
  transport: McpTransport;
  config?: McpServerConfig;
  enabled: boolean;
  description?: string | null;
  metadata?: Record<string, string>;
}

export interface CreateMcpServerInput {
  name: string;
  transport: McpTransport;
  command?: string | null;
  args?: string[] | null;
  url?: string | null;
  enabled?: boolean | null;
  description?: string | null;
  agent_id?: string | null;
  skill_id?: string | null;
}

// ---------------------------------------------------------------------------
// Memory (Fase 11)
// ---------------------------------------------------------------------------

/** Entrada de memória. `agent_id` null => compartilhada do workspace. */
export interface MemoryEntry {
  id: string;
  workspace_id: string;
  agent_id?: string | null;
  category: string;
  content: string;
  metadata?: Record<string, string>;
  timestamp: number;
}

/** Payload de criação de memória. `agent_id` ausente => compartilhada. */
export interface CreateMemoryInput {
  agent_id?: string | null;
  category: string;
  content: string;
  metadata?: Record<string, string>;
}

// ---------------------------------------------------------------------------
// Workflow chain + events (Fases 6/7)
// ---------------------------------------------------------------------------

export type ChainStatus = "idle" | "running" | "completed" | "failed";

export interface ChainDelivery {
  source: string;
  target: string;
  delivered: boolean;
  error?: string | null;
}

export type WorkflowEventType =
  | "workflow_started"
  | "step_started"
  | "message_delivered"
  | "step_failed"
  | "workflow_completed"
  | "workflow_failed";

export interface WorkflowEvent {
  id: string;
  workspace_id: string;
  workflow_id: string;
  event: WorkflowEventType;
  agent_id?: string | null;
  step?: string | null;
  data?: string | null;
  timestamp: number;
}

export interface ChainExecution {
  id: string;
  workspace_id: string;
  status: ChainStatus;
  order: string[];
  deliveries: ChainDelivery[];
  events: WorkflowEvent[];
  error?: string | null;
}

// ---------------------------------------------------------------------------
// Permissions / Security (Fase 17) + Computer Control (Fase 16)
// ---------------------------------------------------------------------------

export type Permission =
  | "browser_control"
  | "app_control"
  | "shell"
  | "network"
  | "filesystem";

export interface PermissionGrant {
  workspace_id: string;
  agent_id: string;
  permission: Permission;
}

export type ComputerActionKind =
  | "navigate"
  | "click"
  | "type_text"
  | "screenshot"
  | "launch_app"
  | "tap";

export interface ComputerAction {
  action: ComputerActionKind;
  target: string;
}

export interface ComputerActionResult {
  permitted: boolean;
  executed: boolean;
  message: string;
}

// ---------------------------------------------------------------------------
// History / Replay (Fase 18)
// ---------------------------------------------------------------------------

export interface HistoryEntry {
  id: string;
  workspace_id: string;
  agent_id: string;
  direction: string;
  data: string;
  timestamp: number;
}

// ---------------------------------------------------------------------------
// Templates (Fase 19)
// ---------------------------------------------------------------------------

export interface AgentTemplate {
  id: string;
  name: string;
  description: string;
  role: Role;
  runtime: Runtime;
  kind: AgentKind;
  model: string;
  command: string;
  args: string[];
  working_dir: string;
  auto_start: boolean;
}

export interface CreateTemplateInput {
  name: string;
  description?: string;
  role: Role;
  runtime: Runtime;
  kind?: AgentKind;
  model?: string;
  command?: string;
  args?: string[];
  working_dir?: string;
  auto_start?: boolean;
}