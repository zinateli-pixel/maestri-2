/**
 * Registry de sinks de output de terminal, fora do Zustand.
 *
 * Cada agente pode ter no máximo um sink registrado (o terminal montado
 * para aquele agente). O listener global de `agent_output` encaminha os
 * chunks diretamente para o sink, sem passar pelo estado React — assim
 * nenhum re-render é disparado a cada chunk de PTY.
 *
 * Se nenhum terminal estiver montado para um agentId, o chunk é
 * descartado silenciosamente (o processo continua rodando no backend).
 */

export type OutputSink = (data: string) => void;

const sinks = new Map<string, OutputSink>();

/** Registra o sink de output de um agente. Substitui o anterior, se houver. */
export function registerSink(agentId: string, sink: OutputSink): void {
  sinks.set(agentId, sink);
}

/** Remove o sink de um agente (no unmount do terminal). */
export function unregisterSink(agentId: string, sink: OutputSink): void {
  // Só remove se o sink registrado ainda for o mesmo (evita remover o sink
  // de uma nova instância montada em seguida, ex.: React StrictMode).
  if (sinks.get(agentId) === sink) {
    sinks.delete(agentId);
  }
}

/** Encaminha um chunk de output ao sink do agente, se existir. */
export function routeOutput(agentId: string, data: string): void {
  const sink = sinks.get(agentId);
  if (sink) {
    sink(data);
  }
}
