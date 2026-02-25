import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { CommandResult, AppSettings, DeviceIoConfig } from "../../types";

const AVAILABLE_ALGORITHMS = [
  { id: "XXH64", label: "XXH64", desc: "Ultra-fast, recommended default" },
  { id: "XXH3", label: "XXH3", desc: "Next-gen, fastest available" },
  { id: "XXH128", label: "XXH128", desc: "128-bit XXH variant" },
  { id: "SHA256", label: "SHA-256", desc: "Cryptographic, high security" },
  { id: "MD5", label: "MD5", desc: "Legacy compatibility" },
];

const DEVICE_TYPES: { key: keyof AppSettings["ioScheduling"]; label: string; icon: string }[] = [
  { key: "hdd", label: "HDD", icon: "💿" },
  { key: "ssd", label: "SSD", icon: "⚡" },
  { key: "nvme", label: "NVMe", icon: "🚀" },
  { key: "raid", label: "RAID", icon: "🏗️" },
  { key: "network", label: "Network", icon: "🌐" },
];

export function SettingsView() {
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadSettings = useCallback(async () => {
    try {
      const result = await invoke<CommandResult<AppSettings>>("get_settings");
      if (result.success && result.data) {
        setSettings(result.data);
      }
    } catch (err) {
      console.error("Failed to load settings:", err);
    }
  }, []);

  useEffect(() => {
    loadSettings();
  }, [loadSettings]);

  const handleSave = async () => {
    if (!settings) return;
    setSaving(true);
    setSaved(false);
    setError(null);
    try {
      const result = await invoke<CommandResult<boolean>>("save_settings", {
        settings,
      });
      if (result.success) {
        setSaved(true);
        setTimeout(() => setSaved(false), 2000);
      } else {
        setError(result.error || "Failed to save settings");
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  };

  const toggleAlgorithm = (algoId: string) => {
    if (!settings) return;
    const current = settings.hashAlgorithms;
    const next = current.includes(algoId)
      ? current.filter((a) => a !== algoId)
      : [...current, algoId];
    // Must have at least one algorithm
    if (next.length === 0) return;
    setSettings({ ...settings, hashAlgorithms: next });
  };

  const updateOffload = (key: string, value: boolean | number) => {
    if (!settings) return;
    setSettings({
      ...settings,
      offload: { ...settings.offload, [key]: value },
    });
  };

  const updateIo = (
    deviceKey: keyof AppSettings["ioScheduling"],
    field: keyof DeviceIoConfig,
    value: number
  ) => {
    if (!settings) return;
    setSettings({
      ...settings,
      ioScheduling: {
        ...settings.ioScheduling,
        [deviceKey]: {
          ...settings.ioScheduling[deviceKey],
          [field]: value,
        },
      },
    });
  };

  const updateEmail = (key: string, value: string | number | boolean) => {
    if (!settings) return;
    setSettings({
      ...settings,
      email: { ...settings.email, [key]: value },
    });
  };

  if (!settings) {
    return (
      <div className="settings-view">
        <div className="view-header">
          <h2>Settings</h2>
        </div>
        <div className="empty-state">
          <p>Loading settings...</p>
        </div>
      </div>
    );
  }

  return (
    <div className="settings-view">
      <div className="view-header">
        <h2>Settings</h2>
        <div className="settings-actions">
          {saved && <span className="save-success">&#x2713; Saved</span>}
          <button
            className="btn-primary"
            onClick={handleSave}
            disabled={saving}
          >
            {saving ? "Saving..." : "Save Settings"}
          </button>
        </div>
      </div>

      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button onClick={() => setError(null)}>Dismiss</button>
        </div>
      )}

      <div className="settings-sections">
        {/* ─── Hash Algorithms ──────────────────────────────────── */}
        <section className="settings-section">
          <h3>Hash Algorithms</h3>
          <p>Select which hash algorithms to use during copy verification.</p>
          <div className="algo-grid">
            {AVAILABLE_ALGORITHMS.map((algo) => (
              <label
                key={algo.id}
                className={`algo-chip ${settings.hashAlgorithms.includes(algo.id) ? "algo-chip--active" : ""}`}
              >
                <input
                  type="checkbox"
                  checked={settings.hashAlgorithms.includes(algo.id)}
                  onChange={() => toggleAlgorithm(algo.id)}
                />
                <span className="algo-name">{algo.label}</span>
                <span className="algo-desc">{algo.desc}</span>
              </label>
            ))}
          </div>
        </section>

        {/* ─── Offload Defaults ─────────────────────────────────── */}
        <section className="settings-section">
          <h3>Offload Defaults</h3>
          <p>Default options applied to every new offload job.</p>
          <div className="settings-grid">
            <label className="toggle-row">
              <span className="toggle-label">Source Verification</span>
              <span className="toggle-desc">
                Hash source files before copying to detect read errors
              </span>
              <input
                type="checkbox"
                className="toggle-input"
                checked={settings.offload.sourceVerify}
                onChange={(e) =>
                  updateOffload("sourceVerify", e.target.checked)
                }
              />
              <span className="toggle-switch" />
            </label>

            <label className="toggle-row">
              <span className="toggle-label">Post-Copy Verification</span>
              <span className="toggle-desc">
                Re-read destination files and verify hashes match
              </span>
              <input
                type="checkbox"
                className="toggle-input"
                checked={settings.offload.postVerify}
                onChange={(e) =>
                  updateOffload("postVerify", e.target.checked)
                }
              />
              <span className="toggle-switch" />
            </label>

            <label className="toggle-row">
              <span className="toggle-label">Generate ASC MHL</span>
              <span className="toggle-desc">
                Create chain-of-custody manifest after successful copy
              </span>
              <input
                type="checkbox"
                className="toggle-input"
                checked={settings.offload.generateMhl}
                onChange={(e) =>
                  updateOffload("generateMhl", e.target.checked)
                }
              />
              <span className="toggle-switch" />
            </label>

            <div className="number-row">
              <span className="toggle-label">Buffer Size</span>
              <span className="toggle-desc">
                IO buffer size for file operations
              </span>
              <select
                className="settings-select"
                value={settings.offload.bufferSize}
                onChange={(e) =>
                  updateOffload("bufferSize", Number(e.target.value))
                }
              >
                <option value={1048576}>1 MB</option>
                <option value={2097152}>2 MB</option>
                <option value={4194304}>4 MB (default)</option>
                <option value={8388608}>8 MB</option>
                <option value={16777216}>16 MB</option>
              </select>
            </div>

            <div className="number-row">
              <span className="toggle-label">Max Retries</span>
              <span className="toggle-desc">
                Number of retry attempts for failed file copies
              </span>
              <select
                className="settings-select"
                value={settings.offload.maxRetries}
                onChange={(e) =>
                  updateOffload("maxRetries", Number(e.target.value))
                }
              >
                <option value={0}>0 (no retry)</option>
                <option value={1}>1</option>
                <option value={2}>2</option>
                <option value={3}>3 (default)</option>
                <option value={5}>5</option>
              </select>
            </div>
          </div>
        </section>

        {/* ─── IO Scheduling ───────────────────────────────────── */}
        <section className="settings-section">
          <h3>IO Scheduling</h3>
          <p>
            Per-device concurrency settings. Lower values protect HDDs from
            excessive seeking. Higher values utilize fast SSD/NVMe bandwidth.
          </p>
          <div className="io-table">
            <div className="io-table-header">
              <span>Device</span>
              <span>Max Concurrent</span>
              <span>Buffer (MB)</span>
            </div>
            {DEVICE_TYPES.map((dt) => {
              const cfg = settings.ioScheduling[dt.key];
              return (
                <div key={dt.key} className="io-table-row">
                  <span className="io-device">
                    <span className="io-device-icon">{dt.icon}</span>
                    {dt.label}
                  </span>
                  <input
                    type="number"
                    className="settings-number"
                    min={1}
                    max={32}
                    value={cfg.maxConcurrent}
                    onChange={(e) =>
                      updateIo(
                        dt.key,
                        "maxConcurrent",
                        Math.max(1, Math.min(32, Number(e.target.value)))
                      )
                    }
                  />
                  <input
                    type="number"
                    className="settings-number"
                    min={1}
                    max={64}
                    value={cfg.bufferSizeMb}
                    onChange={(e) =>
                      updateIo(
                        dt.key,
                        "bufferSizeMb",
                        Math.max(1, Math.min(64, Number(e.target.value)))
                      )
                    }
                  />
                </div>
              );
            })}
          </div>
        </section>

        {/* ─── Email Notifications ─────────────────────────────── */}
        <section className="settings-section">
          <h3>Email Notifications</h3>
          <p>Send email alerts when offload jobs complete or fail.</p>

          <label className="toggle-row" style={{ marginBottom: 12 }}>
            <span className="toggle-label">Enable Email Notifications</span>
            <span className="toggle-desc">
              Send alerts via SMTP when jobs finish
            </span>
            <input
              type="checkbox"
              className="toggle-input"
              checked={settings.email.enabled}
              onChange={(e) => updateEmail("enabled", e.target.checked)}
            />
            <span className="toggle-switch" />
          </label>

          {settings.email.enabled && (
            <div className="email-fields">
              <div className="field-row">
                <label className="field-label">SMTP Host</label>
                <input
                  type="text"
                  className="settings-input"
                  placeholder="smtp.gmail.com"
                  value={settings.email.smtpHost}
                  onChange={(e) => updateEmail("smtpHost", e.target.value)}
                />
              </div>
              <div className="field-row-inline">
                <div className="field-row">
                  <label className="field-label">Port</label>
                  <input
                    type="number"
                    className="settings-number"
                    value={settings.email.smtpPort}
                    onChange={(e) =>
                      updateEmail("smtpPort", Number(e.target.value))
                    }
                  />
                </div>
                <label className="toggle-row-compact">
                  <span>TLS</span>
                  <input
                    type="checkbox"
                    className="toggle-input"
                    checked={settings.email.useTls}
                    onChange={(e) => updateEmail("useTls", e.target.checked)}
                  />
                  <span className="toggle-switch" />
                </label>
              </div>
              <div className="field-row">
                <label className="field-label">Username</label>
                <input
                  type="text"
                  className="settings-input"
                  placeholder="user@gmail.com"
                  value={settings.email.smtpUsername}
                  onChange={(e) =>
                    updateEmail("smtpUsername", e.target.value)
                  }
                />
              </div>
              <div className="field-row">
                <label className="field-label">From Address</label>
                <input
                  type="email"
                  className="settings-input"
                  placeholder="dit-system@studio.com"
                  value={settings.email.fromAddress}
                  onChange={(e) =>
                    updateEmail("fromAddress", e.target.value)
                  }
                />
              </div>
              <div className="field-row">
                <label className="field-label">To Address</label>
                <input
                  type="email"
                  className="settings-input"
                  placeholder="team@studio.com"
                  value={settings.email.toAddress}
                  onChange={(e) => updateEmail("toAddress", e.target.value)}
                />
              </div>
            </div>
          )}
        </section>
      </div>
    </div>
  );
}
