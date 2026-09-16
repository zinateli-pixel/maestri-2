import { useCallback, useEffect, useRef, useState } from "react";
import { Terminal as XTerm } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import "@xterm/xterm/css/xterm.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { registerSink, unregisterSink } from "../terminal/terminalRegistry";
import { useWorkspaceStore } from "../store/workspaceStore";
import type { WorkspaceState } from "../types/models";
import { createPortal } from "react-dom";

interface TerminalProps {
  /** Id do agente dono deste terminal (roteia output e input). */
  agentId: string;
}

const MIN_FONT = 8;
const MAX_FONT = 24;
const DEFAULT_FONT = 12;

/**
 * Terminal real (xterm.js) conectado ao PTY do agente.
 *
 * - Output: terminalRegistry → term.write() (ANSI/VT100 nativo).
 * - Input: onData → send_agent_input (Ctrl+C, setas, backspace...).
 * - Resize: fit() → resize_agent(cols, rows).
 * - Scroll: wheel é capturado pelo xterm (stopPropagation nativo) —
 *   o canvas NÃO faz zoom quando o pointer está sobre o terminal.
 *
 * NÃO inicia nem mata o processo; apenas conecta-se ao stream do agente.
 */
export function Terminal({ agentId }: TerminalProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<XTerm | null>(null);
  const fitRef = useRef<(() => void) | null>(null);
  const searchRef = useRef<SearchAddon | null>(null);
  const [fontSize, setFontSize] = useState(DEFAULT_FONT);
  const [menuOpen, setMenuOpen] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [fullscreen, setFullscreen] = useState(false);

  // Chave que força recriação do terminal quando o processo reinicia.
  // Incrementa a cada transição para "starting".
  const [terminalKey, setTerminalKey] = useState(0);

  // Cria/Recria a instância do xterm. Chamada no mount e a cada novo processo.
  const createTerminal = useCallback(() => {
    const container = containerRef.current;
    if (!container) return;

    // Descarta instância anterior se existir
    if (termRef.current) {
      try {
        termRef.current.dispose();
      } catch {}
      termRef.current = null;
    }
    if (searchRef.current) {
      searchRef.current = null;
    }

    const term = new XTerm({
      cursorBlink: true,
      fontSize,
      fontFamily: "var(--font-mono)",
      scrollback: 5000,
      theme: {
        background: "var(--color-bg-deep)",
        foreground: "var(--color-text-secondary)",
        cursor: "var(--color-accent-primary)",
        selectionBackground: "var(--color-border-default)",
      },
      allowProposedApi: true,
    });

    const fit = new FitAddon();
    const search = new SearchAddon();
    term.loadAddon(fit);
    term.loadAddon(search);
    term.open(container);

    termRef.current = term;
    searchRef.current = search;

    const fitAndResize = () => {
      try {
        fit.fit();
        const { cols, rows } = term;
        if (cols > 0 && rows > 0) {
          void invoke("resize_agent", { id: agentId, cols, rows }).catch(
            () => {}
          );
        }
      } catch {
        // Container sem dimensões ainda; o ResizeObserver tentará de novo.
      }
    };

    const sink = (data: string) => {
      term.write(data);
    };
    registerSink(agentId, sink);

    // xterm emite texto colado e Enter em eventos separados. Invokes Tauri
    // concorrentes podem chegar ao Rust fora de ordem, fazendo o CR ultrapassar
    // o texto e quebrar o buffer de intenção. A fila preserva exatamente a
    // ordem observada no terminal para texto normal e intenções nativas.
    let inputQueue: Promise<void> = Promise.resolve();
    const dataDisposable = term.onData((data) => {
      inputQueue = inputQueue
        .then(async () => {
          const connectionFeedback = await invoke<string | null>("send_agent_input", {
            id: agentId,
            input: data,
          });
          if (!connectionFeedback) return;

          // A native connection intent mutates topology in Rust. Refresh only
          // the edge list so the canvas reflects the Maestri-owned connection
          // without clobbering local node geometry or other in-flight edits.
          const latest = await invoke<WorkspaceState>("get_workspace_state");
          const current = useWorkspaceStore.getState().state;
          if (current?.metadata.id === latest.metadata.id) {
            useWorkspaceStore.setState({ state: { ...current, edges: latest.edges } });
          }
          useWorkspaceStore.getState().pushToast(connectionFeedback, "success");
        })
        // Um erro de input não pode inutilizar permanentemente a fila: o
        // próximo evento ainda precisa ser entregue ao processo.
        .catch(() => {});
    });

    // CRÍTICO: captura o wheel ANTES do React Flow (d3-zoom escuta no pane).
    // Listener nativo com stopPropagation impede que o canvas faça zoom
    // quando o pointer está sobre o terminal.
    const onWheel = (e: WheelEvent) => {
      e.stopPropagation();
    };
    container.addEventListener("wheel", onWheel, { passive: true });

    // Fit inicial (após o primeiro paint) + fit ao redimensionar a janela.
    // O resize do NODE é dirigido pelo store (ver efeito abaixo) — não
    // usamos ResizeObserver aqui para evitar o loop fit→layout→observer.
    const raf = requestAnimationFrame(fitAndResize);
    const onWindowResize = () => {
      requestAnimationFrame(fitAndResize);
    };
    window.addEventListener("resize", onWindowResize);

    // Expõe o fit para o efeito de dimensões (fora deste closure).
    fitRef.current = fitAndResize;

    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("resize", onWindowResize);
      container.removeEventListener("wheel", onWheel);
      dataDisposable.dispose();
      unregisterSink(agentId, sink);
      term.dispose();
      termRef.current = null;
      searchRef.current = null;
      fitRef.current = null;
    };
  }, [agentId, fontSize]);

  // Efeito principal: cria terminal no mount e a cada novo processo (terminalKey muda)
  useEffect(() => {
    createTerminal();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentId, terminalKey, createTerminal]);

  // Re-envia fit/resize ao PTY assim que o processo estiver registrado no
  // backend. O evento `agent_started` é emitido APÓS o handle entrar no mapa
  // de processos, então o `resize_agent` passa a ser aplicado de fato. Antes
  // disso o fit do mount acontecia com o agente parado e o resize era
  // descartado, deixando o PTY no tamanho fixo de 80x24 (causa do bug do
  // Kilo/OpenCode renderizando desproporcional ao node).
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void (async () => {
      try {
        unlisten = await listen("agent_started", (event) => {
          const payload = event.payload as { agentId?: string };
          if (!cancelled && payload?.agentId === agentId) {
            requestAnimationFrame(() => fitRef.current?.());
          }
        });
      } catch {
        // Sem eventos (ex.: preview fora do Tauri) — ignora silenciosamente.
      }
    })();
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [agentId]);

  // Incrementa terminalKey quando o status transiciona para "starting"
  // (novo processo iniciando). Isso força recriação completa do xterm.
  const status = useWorkspaceStore(
    (s) => s.state?.agents.find((a) => a.id === agentId)?.status
  );
  const prevStatusRef = useRef(status);
  useEffect(() => {
    const prev = prevStatusRef.current;
    prevStatusRef.current = status;
    // Nova instância xterm a cada novo processo (transição para "starting")
    if (status === "starting" && prev !== "starting") {
      setTerminalKey((k) => k + 1);
    }
  }, [status]);

  // Quando o node muda de tamanho (NodeResizer → store), refaz o fit.
  const agentWidth = useWorkspaceStore(
    (s) => s.state?.agents.find((a) => a.id === agentId)?.width
  );
  const agentHeight = useWorkspaceStore(
    (s) => s.state?.agents.find((a) => a.id === agentId)?.height
  );
  useEffect(() => {
    if (agentWidth == null && agentHeight == null) return;
    const raf = requestAnimationFrame(() => fitRef.current?.());
    return () => cancelAnimationFrame(raf);
  }, [agentWidth, agentHeight]);

  // Aplica mudança de fonte na instância atual do terminal.
  useEffect(() => {
    const term = termRef.current;
    if (term) {
      term.options.fontSize = fontSize;
    }
  }, [fontSize, terminalKey]);

  // Cmd/Ctrl+F abre busca quando este terminal está focado.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "f") {
        const term = termRef.current;
        if (term && term.element?.contains(document.activeElement)) {
          e.preventDefault();
          setSearchOpen(true);
        }
      }
      if (e.key === "Escape") {
        setSearchOpen(false);
        setMenuOpen(false);
        if (fullscreen) setFullscreen(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [fullscreen]);

  const doSearch = (q: string) => {
    setSearchQuery(q);
    if (q) {
      searchRef.current?.findNext(q);
    } else {
      searchRef.current?.clearDecorations();
    }
  };

  const menuAction = (action: string) => {
    const term = termRef.current;
    setMenuOpen(false);
    if (!term) return;
    switch (action) {
      case "clear":
        term.clear();
        break;
      case "copy": {
        const sel = term.getSelection();
        if (sel) void navigator.clipboard.writeText(sel);
        break;
      }
      case "paste":
        void navigator.clipboard.readText().then((text) => {
          if (text) term.paste(text);
        });
        break;
      case "selectAll":
        term.selectAll();
        break;
      case "search":
        setSearchOpen(true);
        break;
      case "fontUp":
        setFontSize((s) => Math.min(MAX_FONT, s + 1));
        break;
      case "fontDown":
        setFontSize((s) => Math.max(MIN_FONT, s - 1));
        break;
      case "fontReset":
        setFontSize(DEFAULT_FONT);
        break;
      case "scrollBottom":
        term.scrollToBottom();
        break;
      case "fullscreen":
        setFullscreen((f) => !f);
        break;
    }
  };

  // Render fullscreen as portal
  const fullscreenContent = fullscreen && termRef.current ? (
    createPortal(
      <div className="terminal-fullscreen" style={{ display: "flex", flexDirection: "column" }}>
        {/* Fullscreen header */}
        <div
          className="nodrag"
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            padding: "var(--space-3) var(--space-4)",
            background: "var(--color-bg-elevated)",
            borderBottom: "1px solid var(--color-border-subtle)",
            flexShrink: 0,
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: "var(--space-3)" }}>
            <span
              style={{
                width: 10,
                height: 10,
                borderRadius: "var(--radius-full)",
                background: "var(--color-status-running)",
                boxShadow: "0 0 8px var(--color-status-running)",
              }}
            />
            <span style={{ fontWeight: "var(--font-weight-bold)", fontSize: "var(--text-md)" }}>
              {agentId}
            </span>
            <span
              style={{
                fontSize: "var(--text-xs)",
                color: "var(--color-text-subtle)",
                fontFamily: "var(--font-mono)",
              }}
            >
              PTY connected
            </span>
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
            <button
              className="icon-btn"
              onClick={() => setFullscreen(false)}
              title="Exit Fullscreen (Esc)"
              aria-label="Exit Fullscreen"
            >
              ✕
            </button>
          </div>
        </div>
        {/* Terminal area */}
        <div
          ref={containerRef}
          className="nodrag nopan"
          style={{ flex: 1, minHeight: 0, overflow: "hidden" }}
        />
      </div>,
      document.body
    )
  ) : null;

  return (
    <>
      {fullscreenContent}
      <div
        className={fullscreen ? "terminal-fullscreen" : undefined}
        style={{
          position: "relative",
          width: "100%",
          height: "100%",
          display: "flex",
          flexDirection: "column",
          background: "var(--color-bg-deep)",
        }}
      >
        {/* Barra de busca (Cmd/Ctrl+F) */}
        {searchOpen && (
          <div
            className="nodrag"
            style={{
              display: "flex",
              gap: "var(--space-2)",
              padding: "var(--space-2) var(--space-3)",
              borderBottom: "1px solid var(--color-border-subtle)",
              background: "var(--color-bg-elevated)",
            }}
          >
            <input
              autoFocus
              value={searchQuery}
              onChange={(e) => doSearch(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") setSearchOpen(false);
                if (e.key === "Enter") {
                  searchRef.current?.findNext(searchQuery);
                }
              }}
              placeholder="Buscar no terminal…"
              style={{
                flex: 1,
                background: "var(--color-bg-input)",
                border: "1px solid var(--color-border-default)",
                borderRadius: "var(--radius-md)",
                padding: "var(--space-1) var(--space-2)",
                color: "var(--color-text-primary)",
                fontSize: "var(--text-sm)",
                outline: "none",
              }}
            />
            <button
              className="btn btn-ghost btn-sm"
              onClick={() => setSearchOpen(false)}
            >
              ✕
            </button>
          </div>
        )}

        {/* Área do xterm */}
        <div
          ref={containerRef}
          className="nodrag nopan"
          style={{ flex: 1, minHeight: 0, overflow: "hidden" }}
        />

        {/* Menu ⋯ discreto no canto superior direito */}
        <div
          className="nodrag"
          style={{ position: "absolute", top: "var(--space-2)", right: "var(--space-2)", zIndex: 10 }}
        >
          <button
            className="icon-btn"
            onClick={() => setMenuOpen((o) => !o)}
            title="Opções do terminal"
            aria-label="Opções do terminal"
            aria-expanded={menuOpen}
            aria-haspopup="true"
          >
            ⋯
          </button>
          {menuOpen && (
            <div className="context-menu" style={{ minWidth: 180 }}>
              {[
                ["clear", "Clear"],
                ["copy", "Copy"],
                ["paste", "Paste"],
                ["selectAll", "Select All"],
                ["search", "Search  ⌘F"],
                ["fontUp", "Font +"],
                ["fontDown", "Font −"],
                ["fontReset", "Reset Font"],
                ["scrollBottom", "Scroll to Bottom"],
                ["fullscreen", fullscreen ? "Exit Fullscreen" : "Fullscreen"],
              ].map(([action, label]) => (
                <button
                  key={action}
                  className="context-menu-item"
                  onClick={() => menuAction(action)}
                  role="menuitem"
                >
                  {label}
                </button>
              ))}
            </div>
          )}
        </div>
      </div>
    </>
  );
}
