import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { attachEventListeners } from "./store/workspaceStore";

// "ResizeObserver loop completed with undelivered notifications" é um aviso
// benigno da especificação (ocorre quando um callback de RO altera layout).
// Usamos CAPTURE phase para interceptar antes do cliente de dev do Vite.
window.addEventListener(
  "error",
  (e) => {
    if (e.message?.includes("ResizeObserver loop")) {
      e.stopImmediatePropagation();
      e.preventDefault();
    }
  },
  true
);

// Registra os listeners de eventos do backend (agent_output, agent_started, etc.)
// uma única vez, antes de renderizar.
attachEventListeners();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
