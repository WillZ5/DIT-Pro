import { useState, useEffect, useCallback } from "react";
import { safeInvoke } from "../../utils/tauriCompat";
import { useI18n, type Locale } from "../../i18n";
import type { CommandResult, AppSettings, DeviceIoConfig } from "../../types";

export function SettingsView() {
  const { t, locale, setLocale } = useI18n();

  const AVAILABLE_ALGORITHMS = [
    { id: "XXH64", label: "XXH64", desc: t.settings.algoXxh64Desc },
    { id: "XXH3", label: "XXH3", desc: t.settings.algoXxh3Desc },
    { id: "XXH128", label: "XXH128", desc: t.settings.algoXxh128Desc },
    { id: "SHA256", label: "SHA-256", desc: t.settings.algoSha256Desc },
    { id: "MD5", label: "MD5", desc: t.settings.algoMd5Desc },
  ];

  const DEVICE_TYPES: { key: keyof AppSettings["ioScheduling"]; label: string; desc: string }[] = [
    { key: "hdd", label: "HDD", desc: t.settings.deviceHdd },
    { key: "ssd", label: "SSD", desc: t.settings.deviceSsd },
    { key: "nvme", label: "NVMe", desc: t.settings.deviceNvme },
    { key: "raid", label: "RAID", desc: t.settings.deviceRaid },
    { key: "network", label: "Network", desc: t.settings.deviceNetwork },
  ];

  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadSettings = useCallback(async () => {
    try {
      const result = await safeInvoke<CommandResult<AppSettings>>("get_settings");
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
      const result = await safeInvoke<CommandResult<boolean>>("save_settings", {
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
          <h2>{t.settings.title}</h2>
        </div>
        <div className="empty-state">
          <p>{t.settings.loadingSettings}</p>
        </div>
      </div>
    );
  }

  return (
    <div className="settings-view">
      <div className="view-header">
        <h2>{t.settings.title}</h2>
        <div className="settings-actions">
          {saved && <span className="save-success">{t.common.saved}</span>}
          <button
            className="btn-primary"
            onClick={handleSave}
            disabled={saving}
          >
            {saving ? t.common.saving : t.settings.saveSettings}
          </button>
        </div>
      </div>

      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button onClick={() => setError(null)}>{t.common.dismiss}</button>
        </div>
      )}

      <div className="settings-sections">
        {/* Language */}
        <section className="settings-section">
          <h3>{t.settings.languageTitle}</h3>
          <p>{t.settings.languageDesc}</p>
          <div className="algo-grid algo-grid--compact">
            {([
              { id: "en" as Locale, label: t.settings.languageEn },
              { id: "zh" as Locale, label: t.settings.languageZh },
            ]).map((lang) => (
              <label
                key={lang.id}
                className={`algo-chip ${locale === lang.id ? "algo-chip--active" : ""}`}
              >
                <input
                  type="radio"
                  name="language"
                  checked={locale === lang.id}
                  onChange={() => setLocale(lang.id)}
                />
                <span className="algo-name">{lang.label}</span>
              </label>
            ))}
          </div>
        </section>

        {/* Hash Algorithms */}
        <section className="settings-section">
          <h3>{t.settings.hashAlgorithmsTitle}</h3>
          <p>{t.settings.hashAlgorithmsDesc}</p>
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

        {/* Offload Defaults */}
        <section className="settings-section">
          <h3>{t.settings.offloadDefaultsTitle}</h3>
          <p>{t.settings.offloadDefaultsDesc}</p>
          <div className="settings-grid">
            <label className="toggle-row">
              <span className="toggle-label">{t.settings.sourceVerification}</span>
              <span className="toggle-desc">
                {t.settings.sourceVerifyDesc}
              </span>
              <input
                type="checkbox"
                className="toggle-input"
                checked={settings.offload.sourceVerify}
                onChange={(e) => updateOffload("sourceVerify", e.target.checked)}
              />
              <span className="toggle-switch" />
            </label>

            <label className="toggle-row">
              <span className="toggle-label">{t.settings.postCopyVerification}</span>
              <span className="toggle-desc">
                {t.settings.postVerifyDesc}
              </span>
              <input
                type="checkbox"
                className="toggle-input"
                checked={settings.offload.postVerify}
                onChange={(e) => updateOffload("postVerify", e.target.checked)}
              />
              <span className="toggle-switch" />
            </label>

            <label className="toggle-row">
              <span className="toggle-label">{t.settings.generateAscMhl}</span>
              <span className="toggle-desc">
                {t.settings.generateMhlDesc}
              </span>
              <input
                type="checkbox"
                className="toggle-input"
                checked={settings.offload.generateMhl}
                onChange={(e) => updateOffload("generateMhl", e.target.checked)}
              />
              <span className="toggle-switch" />
            </label>

            <label className="toggle-row">
              <span className="toggle-label">{t.settings.cascadingCopy}</span>
              <span className="toggle-desc">
                {t.settings.cascadeDesc}
              </span>
              <input
                type="checkbox"
                className="toggle-input"
                checked={settings.offload.cascade}
                onChange={(e) => updateOffload("cascade", e.target.checked)}
              />
              <span className="toggle-switch" />
            </label>

            <div className="number-row">
              <span className="toggle-label">{t.settings.bufferSize}</span>
              <span className="toggle-desc">{t.settings.bufferSizeDesc}</span>
              <select
                className="settings-select"
                value={settings.offload.bufferSize}
                onChange={(e) => updateOffload("bufferSize", Number(e.target.value))}
              >
                <option value={1048576}>1 MB</option>
                <option value={2097152}>2 MB</option>
                <option value={4194304}>4 MB {t.settings.bufferDefault}</option>
                <option value={8388608}>8 MB</option>
                <option value={16777216}>16 MB</option>
              </select>
            </div>

            <div className="number-row">
              <span className="toggle-label">{t.settings.maxRetries}</span>
              <span className="toggle-desc">{t.settings.maxRetriesDesc}</span>
              <select
                className="settings-select"
                value={settings.offload.maxRetries}
                onChange={(e) => updateOffload("maxRetries", Number(e.target.value))}
              >
                <option value={0}>0 ({t.settings.noRetry})</option>
                <option value={1}>1</option>
                <option value={2}>2</option>
                <option value={3}>3 {t.settings.bufferDefault}</option>
                <option value={5}>5</option>
              </select>
            </div>
          </div>
        </section>

        {/* IO Scheduling */}
        <section className="settings-section">
          <h3>{t.settings.ioSchedulingTitle}</h3>
          <p>{t.settings.ioSchedulingDesc}</p>
          <div className="io-table">
            <div className="io-table-header">
              <span>{t.settings.colDevice}</span>
              <span>{t.settings.colMaxConcurrent}</span>
              <span>{t.settings.colBufferMb}</span>
            </div>
            {DEVICE_TYPES.map((dt) => {
              const cfg = settings.ioScheduling[dt.key];
              return (
                <div key={dt.key} className="io-table-row">
                  <span className="io-device">
                    <span className="io-device-label">{dt.label}</span>
                    <span className="io-device-desc">{dt.desc}</span>
                  </span>
                  <input
                    type="number"
                    className="settings-number"
                    min={1}
                    max={32}
                    value={cfg.maxConcurrent}
                    onChange={(e) =>
                      updateIo(dt.key, "maxConcurrent", Math.max(1, Math.min(32, Number(e.target.value))))
                    }
                  />
                  <input
                    type="number"
                    className="settings-number"
                    min={1}
                    max={64}
                    value={cfg.bufferSizeMb}
                    onChange={(e) =>
                      updateIo(dt.key, "bufferSizeMb", Math.max(1, Math.min(64, Number(e.target.value))))
                    }
                  />
                </div>
              );
            })}
          </div>
        </section>

        {/* Email Notifications */}
        <section className="settings-section">
          <h3>{t.settings.emailTitle}</h3>
          <p>{t.settings.emailDesc}</p>

          <label className="toggle-row" style={{ marginBottom: 12 }}>
            <span className="toggle-label">{t.settings.enableEmail}</span>
            <span className="toggle-desc">{t.settings.enableEmailDesc}</span>
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
                <label className="field-label">{t.settings.smtpHost}</label>
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
                  <label className="field-label">{t.settings.port}</label>
                  <input
                    type="number"
                    className="settings-number"
                    value={settings.email.smtpPort}
                    onChange={(e) => updateEmail("smtpPort", Number(e.target.value))}
                  />
                </div>
                <label className="toggle-row-compact">
                  <span>{t.settings.tls}</span>
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
                <label className="field-label">{t.settings.username}</label>
                <input
                  type="text"
                  className="settings-input"
                  placeholder="user@gmail.com"
                  value={settings.email.smtpUsername}
                  onChange={(e) => updateEmail("smtpUsername", e.target.value)}
                />
              </div>
              <div className="field-row">
                <label className="field-label">{t.settings.fromAddress}</label>
                <input
                  type="email"
                  className="settings-input"
                  placeholder="dit-system@studio.com"
                  value={settings.email.fromAddress}
                  onChange={(e) => updateEmail("fromAddress", e.target.value)}
                />
              </div>
              <div className="field-row">
                <label className="field-label">{t.settings.toAddress}</label>
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
