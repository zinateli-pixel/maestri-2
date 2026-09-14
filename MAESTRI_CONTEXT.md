# MAESTRI 2.0 — Contexto do Projeto

## O que é
IDE desktop (Tauri 2 + React) para orquestração visual de múltiplos agentes de IA
em um canvas. Cada agente é um node com terminal PTY real (xterm.js).

**Princípio arquitetural:** "Runtime ≠ Modelo" — o Runtime (Kilo/ClaudeCode/Custom)
é independente do modelo de dados.

## Stack
- Tauri 2.11.4, React 19.2.8, TypeScript 6.0.3, Vite 8.2.2, pnpm 9.15.9
- @xyflow/react 12.11.6 (React Flow v12), zustand 5.0.15
- @xterm/xterm + addon-fit + addon-search
- Rust edition 2021, portable-pty, serde/serde_json

## Estrutura
```
src/
  App.tsx                      — header, workspace selector, status bar, toasts, auto-save
  App.css                      — design tokens + todos os estilos
  main.tsx                     — attachEventListeners() (1x, guardado)
  store/workspaceStore.ts      — zustand: estado, ações, undo/redo, activity, toasts
  types/models.ts              — espelhos TS dos modelos Rust
  terminal/terminalRegistry.ts — sinks de output fora do React (sem re-render por chunk)
  components/
    canvas/Canvas.tsx          — React Flow: multi-select, search, auto layout, ctx menus
    canvas/AgentNode.tsx       — node com header, terminal, ações
    Terminal.tsx               — xterm + PTY bridge, search, fullscreen
    CommandPalette.tsx         — Cmd+K
    CreateAgentModal.tsx       — templates de runtime
    AgentForm.tsx              — form criar/editar
    AgentDetailPanel.tsx       — inspector lateral
    SettingsPanel.tsx          — settings do workspace (grid/snap/minimap/autosave)
    ActivityPanel.tsx          — log de sessão
src-tauri/src/
  models.rs                    — Agent, Edge, WorkspaceState, enums (serde snake_case)
  state.rs                     — AppState, stop_all_processes(), boot reset de status
  commands.rs                  — todos os #[tauri::command]
  persistence.rs               — JSON multi-workspace + backups (10 últimos)
  process_manager.rs           — 1 processo/agent, eventos Tauri
  runtime.rs                   — RuntimeAdapter trait + LocalShellAdapter (PTY)
  agent_bus.rs                 — barramento interno de mensagens agent↔agent
  context_router.rs            — Agent Protocol (RoutedMessage) + roteamento por edges
  chain.rs                     — Workflow Engine (run_canvas_chain) + eventos
  events.rs                    — WorkflowEvent / WorkflowEventLog
  skills.rs                    — Skills (Fase 9)
  mcp.rs                       — MCP Manager (Fase 10)
  memory.rs                    — Memory (Fase 11)
```

## Features implementadas (Core/IDE completo)
- Canvas: drag, resize (NodeResizer), collapse, lock, multi-select (Shift+click,
  box select, Cmd+A), bulk start/stop/restart/duplicate/delete, context menus
  (node/pane/edge), search (Cmd+F com dim), auto layout grid (Cmd+L), fitView
- Agents: criar (templates), editar, duplicar, deletar (para processo, sem órfãos),
  start/stop/restart/refresh, auto_start
- Terminal: PTY real, input/output, resize, search (Cmd+F no terminal), fullscreen,
  font size, copy/paste
- Workspaces: múltiplos, criar/renomear/duplicar/deletar, seletor no header
- Persistência: JSON por workspace + índice, migração de formato legado
- Undo/redo: histórico agrupado (bulk ops = 1 entrada), limpo ao trocar workspace
- Import/export JSON, backup timestamped (mantém 10 por workspace)
- Settings: grid, snap, minimap, auto-save (intervalo configurável)
- Activity panel: log de sessão com timestamps
- Save state indicator (saved/saving/unsaved)

## Lifecycle (garantias)
- `delete_agent` para o processo antes de remover (sem órfãos)
- `stop_all_processes()` ao trocar/criar/deletar/importar workspace: mata processos
  E reseta status ativos → Stopped (persistido)
- Boot: status Running/Starting/Waiting → Stopped (persiste se mudou)
- `ExitRequested` → `stop_all()` (sem órfãos ao fechar)
- Refresh: kill dropa master PTY antes do join do reader (sem deadlock)
- `saveNow` não substitui state (evita clobber durante drag)
- Listeners Tauri registrados 1x (guarda `listenersAttached`)

## Build & teste
- `pnpm build` — frontend (tsc + vite)
- `cd src-tauri && cargo build` — backend dev
- `cd src-tauri && cargo test` — 148 testes (persistência, modelos, PTY, routing, workflow, skills, MCP, memory, projects, discovery, codex, web/app agents, permissions, computer control, history, templates)
- `pnpm tauri dev` — dev com hot reload
- `pnpm tauri build` — release (.app + .dmg)

## Artefatos de release (etapa 21)
- .app: `src-tauri/target/release/bundle/macos/maestri-2.app` (4.8 MB)
- .dmg: `src-tauri/target/release/bundle/dmg/maestri-2_0.1.0_aarch64.dmg` (2.3 MB)
- Bundle verificado: Info.plist válido, executável presente, sem refs a paths de dev,
  só libs de sistema, quit limpo sem órfãos.

## Fora de escopo / conhecido
- Bug ANSI no terminal: FORA DO ESCOPO (não mexer no pipeline xterm)
- Code splitting do bundle JS (~813 KB)

## Roadmap (role AGENTS.md, itens 1–21)
1..5 — Estabilizar Node/Terminal/PTY, modelo Agent, connections→contexto,
       comunicação agent↔agent, Agent Protocol: CONCLUÍDO (inclui hardening
       da Fase 4: banner de identidade "MAESTRO 2.0" + canvas como fonte de
       verdade, proibição de maestri list/ask/OmniRoute e retry finito, e
       interceptação nativa de intenção de conexão "conecta A ao B").
6..8  — Workflow Engine, eventos/estados, persistência local: CONCLUÍDO.
9..10 — Skills (backend + UI) e MCP Manager (backend + UI): CONCLUÍDO.
11    — Memory: CONCLUÍDO (backend + frontend: types, store e aba no painel).
12    — Workspaces/Projects: CONCLUÍDO (backend + frontend: types, store e
       aba "Projects" no painel de capabilidades).
13    — Runtime Adapters reais (Kilo, Claude Code, OpenCode, Codex) além do
       LocalShellAdapter (Custom): CONCLUÍDO.
14    — Web Agents (AgentKind::Web): CONCLUÍDO. Fronteira de ciclo de vida
       separada do CLI (WebSessionManager + StubWebAgentAdapter), isolamento
       por workspace; driver de navegador fica para a Fase 16.
15    — App Agents (AgentKind::App): CONCLUÍDO (AppSessionManager + stub,
       espelhando a Fase 14).
16    — Computer Control: CONTRATO implementado (catálogo de ações navigate/
       click/type_text/screenshot/launch_app/tap gateado pela permissão, com
       driver stub). O driver real (Playwright/XCTest/adb) é pendente.
17    — Permissions/Security: CONCLUÍDO (deny por padrão, registro persistido
       por workspace; PermissionRegistry + comandos grant/revoke/check).
18    — History/Replay: CONCLUÍDO (histórico persistido de comunicação por
       workspace, comandos list/history_by_agent/clear).
19    — Templates: CONCLUÍDO (AgentTemplate persistido globalmente, list/
       create/delete/apply_template).
20..21 — Cloud/Supabase e Marketplace: BLOQUEADO pelo mandato LOCAL-FIRST
       ("não migre para Supabase" — role AGENTS.md). Exige decisão do
       usuário para seguir.

## Próxima fase (ordem real do roadmap)
1. ⬜ Fase 16 (driver) — integrar um driver REAL de computer control
   (browser/app) atrás do contrato + permissões já existentes.
2. ⬜ Fase 14/15 — substituir os stubs Web/App por execução real.
3. ⬜ Fase 20/21 — Cloud/Supabase e Marketplace (requer liberação do
   mandato local-first pelo usuário).
