import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { CommandResult, WorkflowPreset } from "../../types";

const ALGO_OPTIONS = [
  { id: "XXH64", label: "XXH64" },
  { id: "XXH3", label: "XXH3" },
  { id: "XXH128", label: "XXH128" },
  { id: "SHA256", label: "SHA-256" },
  { id: "MD5", label: "MD5" },
];

const BUFFER_OPTIONS = [
  { value: 1048576, label: "1 MB" },
  { value: 2097152, label: "2 MB" },
  { value: 4194304, label: "4 MB" },
  { value: 8388608, label: "8 MB" },
  { value: 16777216, label: "16 MB" },
];

function emptyPreset(): WorkflowPreset {
  return {
    id: "",
    name: "",
    description: "",
    hashAlgorithms: ["XXH64", "SHA256"],
    sourceVerify: true,
    postVerify: true,
    generateMhl: true,
    bufferSize: 4194304,
    maxRetries: 3,
    cascade: false,
    defaultDestPaths: [],
    createdAt: "",
    updatedAt: "",
  };
}

export function PresetsView() {
  const [presets, setPresets] = useState<WorkflowPreset[]>([]);
  const [editing, setEditing] = useState<WorkflowPreset | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadPresets = useCallback(async () => {
    try {
      const result = await invoke<CommandResult<WorkflowPreset[]>>("list_presets");
      if (result.success && result.data) {
        setPresets(result.data);
      }
    } catch (err) {
      console.error("Failed to load presets:", err);
    }
  }, []);

  useEffect(() => {
    loadPresets();
  }, [loadPresets]);

  const handleNew = () => {
    setEditing(emptyPreset());
    setIsNew(true);
    setError(null);
  };

  const handleEdit = (preset: WorkflowPreset) => {
    setEditing({ ...preset });
    setIsNew(false);
    setError(null);
  };

  const handleCancel = () => {
    setEditing(null);
    setIsNew(false);
    setError(null);
  };

  const handleSave = async () => {
    if (!editing || !editing.name.trim()) {
      setError("Preset name is required");
      return;
    }
    setSaving(true);
    setError(null);

    try {
      const cmd = isNew ? "create_preset" : "update_preset";
      const result = await invoke<CommandResult<WorkflowPreset>>(cmd, {
        presetData: editing,
      });
      if (result.success) {
        await loadPresets();
        setEditing(null);
        setIsNew(false);
      } else {
        setError(result.error || "Failed to save preset");
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (presetId: string) => {
    try {
      const result = await invoke<CommandResult<boolean>>("delete_preset", {
        presetId,
      });
      if (result.success) {
        await loadPresets();
        if (editing?.id === presetId) {
          setEditing(null);
          setIsNew(false);
        }
      } else {
        setError(result.error || "Failed to delete preset");
      }
    } catch (err) {
      setError(String(err));
    }
  };

  const handleDuplicate = (preset: WorkflowPreset) => {
    setEditing({
      ...preset,
      id: "",
      name: `${preset.name} (Copy)`,
      createdAt: "",
      updatedAt: "",
    });
    setIsNew(true);
    setError(null);
  };

  const toggleAlgo = (algoId: string) => {
    if (!editing) return;
    const current = editing.hashAlgorithms;
    const next = current.includes(algoId)
      ? current.filter((a) => a !== algoId)
      : [...current, algoId];
    if (next.length === 0) return;
    setEditing({ ...editing, hashAlgorithms: next });
  };

  const updateField = <K extends keyof WorkflowPreset>(
    key: K,
    value: WorkflowPreset[K]
  ) => {
    if (!editing) return;
    setEditing({ ...editing, [key]: value });
  };

  return (
    <div className="settings-view">
      <div className="view-header">
        <h2>Workflow Presets</h2>
        <div className="settings-actions">
          <button className="btn-primary" onClick={handleNew}>
            + New Preset
          </button>
        </div>
      </div>

      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button onClick={() => setError(null)}>Dismiss</button>
        </div>
      )}

      <div className="presets-layout">
        {/* ─── Preset List ────────────────────────────────────── */}
        <div className="preset-list">
          {presets.length === 0 ? (
            <div className="empty-state">
              <p>No presets yet. Click "+ New Preset" to create one.</p>
            </div>
          ) : (
            presets.map((p) => (
              <div
                key={p.id}
                className={`preset-card ${editing?.id === p.id ? "preset-card--active" : ""}`}
                onClick={() => handleEdit(p)}
              >
                <div className="preset-card-header">
                  <span className="preset-card-name">{p.name}</span>
                  <div className="preset-card-badges">
                    {p.cascade && (
                      <span className="preset-badge preset-badge--cascade">
                        Cascade
                      </span>
                    )}
                    {p.generateMhl && (
                      <span className="preset-badge preset-badge--mhl">MHL</span>
                    )}
                  </div>
                </div>
                {p.description && (
                  <p className="preset-card-desc">{p.description}</p>
                )}
                <div className="preset-card-meta">
                  <span className="preset-card-algos">
                    {p.hashAlgorithms.join(", ")}
                  </span>
                  <span className="preset-card-flags">
                    {[
                      p.sourceVerify && "SrcVerify",
                      p.postVerify && "PostVerify",
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </span>
                </div>
                <div className="preset-card-actions">
                  <button
                    className="btn-small"
                    onClick={(e) => {
                      e.stopPropagation();
                      handleDuplicate(p);
                    }}
                  >
                    Duplicate
                  </button>
                  <button
                    className="btn-small btn-danger"
                    onClick={(e) => {
                      e.stopPropagation();
                      handleDelete(p.id);
                    }}
                  >
                    Delete
                  </button>
                </div>
              </div>
            ))
          )}
        </div>

        {/* ─── Preset Editor ──────────────────────────────────── */}
        {editing && (
          <div className="preset-editor">
            <h3>{isNew ? "New Preset" : "Edit Preset"}</h3>

            <div className="field-row">
              <label className="field-label">Name</label>
              <input
                type="text"
                className="settings-input"
                placeholder="e.g., ARRI Daily Offload"
                value={editing.name}
                onChange={(e) => updateField("name", e.target.value)}
              />
            </div>

            <div className="field-row">
              <label className="field-label">Description</label>
              <input
                type="text"
                className="settings-input"
                placeholder="Optional description..."
                value={editing.description}
                onChange={(e) => updateField("description", e.target.value)}
              />
            </div>

            <div className="field-row">
              <label className="field-label">Hash Algorithms</label>
              <div className="algo-grid algo-grid--compact">
                {ALGO_OPTIONS.map((algo) => (
                  <label
                    key={algo.id}
                    className={`algo-chip ${editing.hashAlgorithms.includes(algo.id) ? "algo-chip--active" : ""}`}
                  >
                    <input
                      type="checkbox"
                      checked={editing.hashAlgorithms.includes(algo.id)}
                      onChange={() => toggleAlgo(algo.id)}
                    />
                    <span className="algo-name">{algo.label}</span>
                  </label>
                ))}
              </div>
            </div>

            <div className="preset-toggles">
              <label className="toggle-row">
                <span className="toggle-label">Source Verification</span>
                <input
                  type="checkbox"
                  className="toggle-input"
                  checked={editing.sourceVerify}
                  onChange={(e) => updateField("sourceVerify", e.target.checked)}
                />
                <span className="toggle-switch" />
              </label>

              <label className="toggle-row">
                <span className="toggle-label">Post-Copy Verification</span>
                <input
                  type="checkbox"
                  className="toggle-input"
                  checked={editing.postVerify}
                  onChange={(e) => updateField("postVerify", e.target.checked)}
                />
                <span className="toggle-switch" />
              </label>

              <label className="toggle-row">
                <span className="toggle-label">Generate ASC MHL</span>
                <input
                  type="checkbox"
                  className="toggle-input"
                  checked={editing.generateMhl}
                  onChange={(e) => updateField("generateMhl", e.target.checked)}
                />
                <span className="toggle-switch" />
              </label>

              <label className="toggle-row">
                <span className="toggle-label">Cascading Copy</span>
                <input
                  type="checkbox"
                  className="toggle-input"
                  checked={editing.cascade}
                  onChange={(e) => updateField("cascade", e.target.checked)}
                />
                <span className="toggle-switch" />
              </label>
            </div>

            <div className="field-row-inline">
              <div className="field-row">
                <label className="field-label">Buffer Size</label>
                <select
                  className="settings-select"
                  value={editing.bufferSize}
                  onChange={(e) =>
                    updateField("bufferSize", Number(e.target.value))
                  }
                >
                  {BUFFER_OPTIONS.map((opt) => (
                    <option key={opt.value} value={opt.value}>
                      {opt.label}
                    </option>
                  ))}
                </select>
              </div>
              <div className="field-row">
                <label className="field-label">Max Retries</label>
                <select
                  className="settings-select"
                  value={editing.maxRetries}
                  onChange={(e) =>
                    updateField("maxRetries", Number(e.target.value))
                  }
                >
                  {[0, 1, 2, 3, 5].map((n) => (
                    <option key={n} value={n}>
                      {n}
                    </option>
                  ))}
                </select>
              </div>
            </div>

            <div className="preset-editor-actions">
              <button className="btn-secondary" onClick={handleCancel}>
                Cancel
              </button>
              <button
                className="btn-primary"
                onClick={handleSave}
                disabled={saving}
              >
                {saving ? "Saving..." : isNew ? "Create Preset" : "Save Changes"}
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
