import { useState } from "react";
import { useWorkspaceStore } from "../store/workspaceStore";
import type { WorkspaceSettings } from "../types/models";

interface SettingsPanelProps {
  onClose: () => void;
}

export function SettingsPanel({ onClose }: SettingsPanelProps) {
  const state = useWorkspaceStore((s) => s.state);
  const updateWorkspaceSettings = useWorkspaceStore((s) => s.updateWorkspaceSettings);
  const pushToast = useWorkspaceStore((s) => s.pushToast);

  const [settings, setSettings] = useState<WorkspaceSettings | null>(
    state?.settings ?? null
  );
  const [saving, setSaving] = useState(false);

  if (!state || !settings) return null;

  const update = <K extends keyof WorkspaceSettings>(key: K, value: WorkspaceSettings[K]) => {
    setSettings((s) => (s ? { ...s, [key]: value } : s));
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      await updateWorkspaceSettings({ id: state.metadata.id, settings });
      onClose();
    } catch (e) {
      pushToast(`Falha ao salvar settings: ${e}`, "error");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="panel-header">
          <h2>Workspace Settings</h2>
          <button className="icon-btn" onClick={onClose} aria-label="Fechar">
            ✕
          </button>
        </div>

        <div className="panel-section">
          <div className="panel-section-title">Canvas</div>

          <label className="field" style={{ flexDirection: "row", alignItems: "center", gap: "var(--space-3)" }}>
            <input
              type="checkbox"
              checked={settings.show_grid}
              onChange={(e) => update("show_grid", e.target.checked)}
              style={{ width: 16, height: 16, accentColor: "var(--color-accent-primary)" }}
            />
            <span style={{ fontSize: "var(--text-sm)" }}>Mostrar grid</span>
          </label>

          <label className="field" style={{ flexDirection: "row", alignItems: "center", gap: "var(--space-3)" }}>
            <input
              type="checkbox"
              checked={settings.snap_to_grid}
              onChange={(e) => update("snap_to_grid", e.target.checked)}
              style={{ width: 16, height: 16, accentColor: "var(--color-accent-primary)" }}
            />
            <span style={{ fontSize: "var(--text-sm)" }}>Snap to grid</span>
          </label>

          <label className="field">
            <span className="field-label">Tamanho do grid (px)</span>
            <input
              className="field-input"
              type="number"
              min={8}
              max={96}
              value={settings.grid_size}
              onChange={(e) => update("grid_size", Math.max(8, Math.min(96, Number(e.target.value) || 24)))}
            />
          </label>

          <label className="field" style={{ flexDirection: "row", alignItems: "center", gap: "var(--space-3)" }}>
            <input
              type="checkbox"
              checked={settings.show_minimap}
              onChange={(e) => update("show_minimap", e.target.checked)}
              style={{ width: 16, height: 16, accentColor: "var(--color-accent-primary)" }}
            />
            <span style={{ fontSize: "var(--text-sm)" }}>Mostrar minimap</span>
          </label>
        </div>

        <div className="panel-divider" />

        <div className="panel-section">
          <div className="panel-section-title">Persistência</div>

          <label className="field" style={{ flexDirection: "row", alignItems: "center", gap: "var(--space-3)" }}>
            <input
              type="checkbox"
              checked={settings.auto_save}
              onChange={(e) => update("auto_save", e.target.checked)}
              style={{ width: 16, height: 16, accentColor: "var(--color-accent-primary)" }}
            />
            <span style={{ fontSize: "var(--text-sm)" }}>Auto-save</span>
          </label>

          <label className="field">
            <span className="field-label">Intervalo de auto-save (segundos)</span>
            <input
              className="field-input"
              type="number"
              min={5}
              max={600}
              value={settings.auto_save_interval}
              onChange={(e) =>
                update("auto_save_interval", Math.max(5, Math.min(600, Number(e.target.value) || 30)))
              }
              disabled={!settings.auto_save}
            />
          </label>
        </div>

        <div className="form-actions">
          <button className="btn btn-ghost" onClick={onClose} disabled={saving}>
            Cancelar
          </button>
          <button className="btn btn-primary" onClick={handleSave} disabled={saving}>
            {saving ? "Salvando…" : "Salvar"}
          </button>
        </div>
      </div>
    </div>
  );
}
