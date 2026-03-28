import { useState, useEffect, useCallback } from "react";
import { safeInvoke, isTauri } from "../../utils/tauriCompat";
import { useI18n, type Locale } from "../../i18n";
import { SystemLog } from "../../components/SystemLog";
import { playChime, type ChimeEvent } from "../../utils/audioChimes";
import { initNotifications, checkNotificationPermission } from "../../utils/notifications";
import type { CommandResult, AppSettings, DeviceIoConfig, SoundSettings } from "../../types";

export function SettingsView() {
  const { t, locale, setLocale } = useI18n();

  const DEVICE_TYPES: { key: keyof AppSettings["ioScheduling"]; label: string; desc: string }[] = [
    { key: "hdd", label: "HDD", desc: t.settings.deviceHdd },
    { key: "ssd", label: "SSD / NVMe", desc: t.settings.deviceSsd },
    { key: "raid", label: "RAID", desc: t.settings.deviceRaid },
    { key: "network", label: "Network", desc: t.settings.deviceNetwork },
  ];

  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [clearLogsDays, setClearLogsDays] = useState(30);
  const [showClearConfirm, setShowClearConfirm] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [exportingBundle, setExportingBundle] = useState(false);
  const [bundlePath, setBundlePath] = useState<string | null>(null);
  const [notifPermission, setNotifPermission] = useState<boolean | null>(null);

  // Check notification permission on mount
  useEffect(() => {
    checkNotificationPermission().then(setNotifPermission);
  }, []);

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

  const updateOffload = (key: string, value: boolean | number | string) => {
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

  const updateCloud = <K extends keyof AppSettings["cloud"]>(key: K, value: AppSettings["cloud"][K]) => {
    if (!settings) return;
    setSettings({
      ...settings,
      cloud: { ...settings.cloud, [key]: value },
    });
  };

  const updateCloudProvider = (key: string, value: string) => {
    if (!settings) return;
    setSettings({
      ...settings,
      cloud: {
        ...settings.cloud,
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        provider: { ...settings.cloud.provider, [key]: value } as any,
      },
    });
  };

  const [testingCloud, setTestingCloud] = useState(false);
  const [cloudTestResult, setCloudTestResult] = useState<{ success: boolean; message: string } | null>(null);

  const testCloudConnection = async () => {
    if (!settings) return;
    setTestingCloud(true);
    setCloudTestResult(null);
    try {
      const result = await safeInvoke<CommandResult<boolean>>("test_cloud_connection", {
        provider: settings.cloud.provider,
      });
      if (result.success) {
        setCloudTestResult({ success: true, message: t.settings.cloudTestSuccess });
      } else {
        setCloudTestResult({ success: false, message: result.error || "Test failed" });
      }
    } catch (err) {
      setCloudTestResult({ success: false, message: String(err) });
    } finally {
      setTestingCloud(false);
    }
  };

  const updateReport = (key: string, value: string | boolean) => {
    if (!settings) return;
    setSettings({
      ...settings,
      report: { ...settings.report, [key]: value },
    });
  };

  const updateSound = (key: keyof SoundSettings, value: boolean | number) => {
    if (!settings) return;
    setSettings({
      ...settings,
      sound: { ...settings.sound, [key]: value },
    });
  };

  const updateNotification = (key: string, value: boolean) => {
    if (!settings) return;
    setSettings({
      ...settings,
      notification: { ...settings.notification, [key]: value },
    });
  };

  const handleTestChime = (event: ChimeEvent) => {
    if (!settings) return;
    // Play with current settings but force the specific event enabled
    playChime(event, { ...settings.sound, [event]: true, enabled: true });
  };

  const handleBrowseExportPath = async () => {
    if (isTauri()) {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const path = await open({ directory: true, title: t.settings.defaultExportPath });
      if (path) updateReport("defaultExportPath", path as string);
    } else {
      updateReport("defaultExportPath", "/Users/demo/DIT-Reports");
    }
  };

  const handleClearLogs = async () => {
    setClearing(true);
    try {
      const result = await safeInvoke<CommandResult<number>>("clear_logs", {
        olderThanDays: clearLogsDays,
      });
      if (result.success) {
        setShowClearConfirm(false);
      } else {
        setError(result.error || "Failed to clear logs");
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setClearing(false);
    }
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

            {settings.offload.cascade && (
              <div className="number-row">
                <span className="toggle-label">{t.settings.cascadeStrategyTitle}</span>
                <span className="toggle-desc">{t.settings.cascadeStrategyDesc}</span>
                <div className="algo-grid algo-grid--compact">
                  {([
                    { id: "speed", label: t.settings.cascadeStrategySpeed },
                    { id: "custom", label: t.settings.cascadeStrategyCustom },
                  ] as const).map((opt) => (
                    <label
                      key={opt.id}
                      className={`algo-chip ${settings.offload.cascadeStrategy === opt.id ? "algo-chip--active" : ""}`}
                    >
                      <input
                        type="radio"
                        name="cascadeStrategy"
                        checked={settings.offload.cascadeStrategy === opt.id}
                        onChange={() => updateOffload("cascadeStrategy", opt.id)}
                      />
                      <span className="algo-name">{opt.label}</span>
                    </label>
                  ))}
                </div>
              </div>
            )}

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

        {/* Report Export */}
        <section className="settings-section">
          <h3>{t.settings.reportTitle}</h3>
          <p>{t.settings.reportDesc}</p>
          <div className="settings-grid">
            <div className="field-row">
              <label className="field-label">{t.settings.defaultExportPath}</label>
              <div className="path-selector">
                <input
                  type="text"
                  className="settings-input"
                  value={settings.report.defaultExportPath}
                  onChange={(e) => updateReport("defaultExportPath", e.target.value)}
                  placeholder="/path/to/reports"
                />
                <button className="btn-secondary" onClick={handleBrowseExportPath}>
                  {t.common.browse}
                </button>
              </div>
            </div>

            <div className="number-row">
              <span className="toggle-label">{t.settings.exportFormat}</span>
              <div className="algo-grid algo-grid--compact">
                {(["html", "txt"] as const).map((fmt) => (
                  <label
                    key={fmt}
                    className={`algo-chip ${settings.report.exportFormat === fmt ? "algo-chip--active" : ""}`}
                  >
                    <input
                      type="radio"
                      name="exportFormat"
                      checked={settings.report.exportFormat === fmt}
                      onChange={() => updateReport("exportFormat", fmt)}
                    />
                    <span className="algo-name">{fmt.toUpperCase()}</span>
                  </label>
                ))}
              </div>
            </div>

            <label className="toggle-row">
              <span className="toggle-label">{t.settings.askPathEachTime}</span>
              <input
                type="checkbox"
                className="toggle-input"
                checked={settings.report.askPathEachTime}
                onChange={(e) => updateReport("askPathEachTime", e.target.checked)}
              />
              <span className="toggle-switch" />
            </label>

            <label className="toggle-row">
              <span className="toggle-label">{t.settings.askFormatEachTime}</span>
              <input
                type="checkbox"
                className="toggle-input"
                checked={settings.report.askFormatEachTime}
                onChange={(e) => updateReport("askFormatEachTime", e.target.checked)}
              />
              <span className="toggle-switch" />
            </label>
          </div>

          {/* Clear Logs */}
          <div style={{ marginTop: 16, paddingTop: 16, borderTop: "1px solid #27272a" }}>
            <h4 style={{ margin: "0 0 4px", fontSize: 13, color: "#a1a1aa" }}>{t.settings.clearLogs}</h4>
            <p style={{ margin: "0 0 8px", fontSize: 12, color: "#71717a" }}>{t.settings.clearLogsDesc}</p>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <span style={{ fontSize: 12, color: "#a1a1aa" }}>{t.settings.clearLogsDays}:</span>
              <select
                className="settings-select"
                value={clearLogsDays}
                onChange={(e) => setClearLogsDays(Number(e.target.value))}
                style={{ width: 80 }}
              >
                <option value={7}>7</option>
                <option value={14}>14</option>
                <option value={30}>30</option>
                <option value={60}>60</option>
                <option value={90}>90</option>
              </select>
              {!showClearConfirm ? (
                <button className="btn-secondary btn-sm" onClick={() => setShowClearConfirm(true)}>
                  {t.settings.clearLogsBtn}
                </button>
              ) : (
                <>
                  <span style={{ fontSize: 12, color: "#ef4444" }}>{t.settings.clearLogsConfirm}</span>
                  <button
                    className="btn-confirm-terminate"
                    style={{ fontSize: 11, padding: "3px 10px" }}
                    onClick={handleClearLogs}
                    disabled={clearing}
                  >
                    {clearing ? t.common.saving : t.settings.clearLogsBtn}
                  </button>
                  <button className="btn-secondary btn-sm" onClick={() => setShowClearConfirm(false)}>
                    {t.common.cancel}
                  </button>
                </>
              )}
            </div>
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
                  placeholder="dit-pro@studio.com"
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

        {/* ─── Sound Notifications ─────────────────────────────── */}
        <section className="settings-section">
          <h3>{t.settings.soundTitle}</h3>
          <p>{t.settings.soundDesc}</p>

          <label className="toggle-row" style={{ marginBottom: 12 }}>
            <span className="toggle-label">{t.settings.soundEnabled}</span>
            <span className="toggle-desc">{t.settings.soundEnabledDesc}</span>
            <input
              type="checkbox"
              className="toggle-input"
              checked={settings.sound.enabled}
              onChange={(e) => updateSound("enabled", e.target.checked)}
            />
            <span className="toggle-switch" />
          </label>

          {settings.sound.enabled && (
            <div className="settings-grid">
              <div className="number-row">
                <span className="toggle-label">{t.settings.soundVolume}</span>
                <div style={{ display: "flex", alignItems: "center", gap: 8, flex: 1 }}>
                  <input
                    type="range"
                    min={0}
                    max={100}
                    value={Math.round(settings.sound.volume * 100)}
                    onChange={(e) => updateSound("volume", Number(e.target.value) / 100)}
                    style={{ flex: 1 }}
                  />
                  <span style={{ fontSize: 12, color: "#a1a1aa", minWidth: 36, textAlign: "right" }}>
                    {Math.round(settings.sound.volume * 100)}%
                  </span>
                </div>
              </div>

              {([
                { key: "taskComplete" as const, label: t.settings.soundTaskComplete, desc: t.settings.soundTaskCompleteDesc, event: "taskComplete" as ChimeEvent },
                { key: "taskFailed" as const, label: t.settings.soundTaskFailed, desc: t.settings.soundTaskFailedDesc, event: "taskFailed" as ChimeEvent },
                { key: "sourceReleased" as const, label: t.settings.soundSourceReleased, desc: t.settings.soundSourceReleasedDesc, event: "sourceReleased" as ChimeEvent },
                { key: "warning" as const, label: t.settings.soundWarning, desc: t.settings.soundWarningDesc, event: "warning" as ChimeEvent },
              ]).map((item) => (
                <div key={item.key} className="toggle-row" style={{ display: "flex", alignItems: "center" }}>
                  <label style={{ display: "contents" }}>
                    <span className="toggle-label">{item.label}</span>
                    <span className="toggle-desc">{item.desc}</span>
                    <input
                      type="checkbox"
                      className="toggle-input"
                      checked={settings.sound[item.key]}
                      onChange={(e) => updateSound(item.key, e.target.checked)}
                    />
                    <span className="toggle-switch" />
                  </label>
                  <button
                    className="btn-secondary btn-sm"
                    style={{ marginLeft: 8, fontSize: 11, padding: "2px 8px" }}
                    onClick={() => handleTestChime(item.event)}
                  >
                    {t.settings.soundTest}
                  </button>
                </div>
              ))}
            </div>
          )}
        </section>

        {/* ─── System Notifications (Push) ────────────────────── */}
        <section className="settings-section">
          <h3>{t.settings.notificationTitle}</h3>
          <p>{t.settings.notificationDesc}</p>

          <label className="toggle-row" style={{ marginBottom: 12 }}>
            <span className="toggle-label">{t.settings.notificationEnabled}</span>
            <span className="toggle-desc">{t.settings.notificationEnabledDesc}</span>
            <input
              type="checkbox"
              className="toggle-input"
              checked={settings.notification.enabled}
              onChange={(e) => updateNotification("enabled", e.target.checked)}
            />
            <span className="toggle-switch" />
          </label>

          {settings.notification.enabled && (
            <div className="settings-grid">
              <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 8 }}>
                <button
                  className="btn-secondary btn-sm"
                  onClick={async () => {
                    const granted = await initNotifications();
                    setNotifPermission(granted);
                  }}
                >
                  {t.settings.notificationPermission}
                </button>
                <span style={{ fontSize: 12, color: notifPermission ? "#4caf50" : "#ef4444" }}>
                  {notifPermission === null ? "—" : notifPermission ? t.settings.notificationGranted : t.settings.notificationDenied}
                </span>
              </div>

              {([
                { key: "taskComplete" as const, label: t.settings.notificationTaskComplete, desc: t.settings.notificationTaskCompleteDesc },
                { key: "taskFailed" as const, label: t.settings.notificationTaskFailed, desc: t.settings.notificationTaskFailedDesc },
                { key: "sourceReleased" as const, label: t.settings.notificationSourceReleased, desc: t.settings.notificationSourceReleasedDesc },
              ]).map((item) => (
                <label key={item.key} className="toggle-row">
                  <span className="toggle-label">{item.label}</span>
                  <span className="toggle-desc">{item.desc}</span>
                  <input
                    type="checkbox"
                    className="toggle-input"
                    checked={settings.notification[item.key]}
                    onChange={(e) => updateNotification(item.key, e.target.checked)}
                  />
                  <span className="toggle-switch" />
                </label>
              ))}
            </div>
          )}
        </section>

        {/* ─── Cloud Synchronization ────────────────────────────────── */}
        <section className="settings-section">
          <h3 className="section-title">{t.settings.cloudSyncTitle}</h3>
          <p className="section-desc">{t.settings.cloudSyncDesc}</p>

          <div className="settings-grid">
            <div className="settings-col">
              <label className="settings-toggle">
                <span className="toggle-label">{t.settings.cloudEnabled}</span>
                <input
                  type="checkbox"
                  className="toggle-input"
                  checked={settings.cloud.enabled}
                  onChange={(e) => updateCloud("enabled", e.target.checked)}
                />
                <span className="toggle-switch" />
              </label>

              {settings.cloud.enabled && (
                <>
                  <div className="form-group" style={{ marginTop: 16 }}>
                    <label>{t.settings.cloudProvider}</label>
                    <select
                      value={settings.cloud.provider.type}
                      onChange={(e) => {
                        const type = e.target.value as "s3" | "webdav";
                        const newProvider = (type === "s3" 
                          ? { type: "s3", endpoint: "", region: "", bucket: "", accessKey: "", secretKey: "" }
                          : { type: "webdav", endpoint: "", username: "", password: "", root: "/" }) as any;
                        updateCloud("provider", newProvider);
                      }}
                    >
                      <option value="webdav">WebDAV (AList / NAS / Baidu / Aliyun)</option>
                      <option value="s3">S3 Compatible (OSS / AWS / R2 / MinIO)</option>
                    </select>
                  </div>

                  {settings.cloud.provider.type === "s3" ? (
                    <div className="proxy-settings-grid" style={{ gridTemplateColumns: "1fr 1fr", background: "none", padding: 0 }}>
                      <div className="form-group">
                        <label>{t.settings.cloudEndpoint}</label>
                        <input type="text" value={(settings.cloud.provider as { endpoint: string }).endpoint || ""} onChange={(e) => updateCloudProvider("endpoint", e.target.value)} placeholder="https://oss-cn-hangzhou.aliyuncs.com" />
                      </div>
                      <div className="form-group">
                        <label>{t.settings.cloudRegion}</label>
                        <input type="text" value={(settings.cloud.provider as { region: string }).region || ""} onChange={(e) => updateCloudProvider("region", e.target.value)} placeholder="cn-hangzhou" />
                      </div>
                      <div className="form-group">
                        <label>{t.settings.cloudBucket}</label>
                        <input type="text" value={(settings.cloud.provider as { bucket: string }).bucket || ""} onChange={(e) => updateCloudProvider("bucket", e.target.value)} />
                      </div>
                      <div className="form-group">
                        <label>{t.settings.cloudAccessKey}</label>
                        <input type="text" value={(settings.cloud.provider as { accessKey: string }).accessKey || ""} onChange={(e) => updateCloudProvider("accessKey", e.target.value)} />
                      </div>
                      <div className="form-group">
                        <label>{t.settings.cloudSecretKey}</label>
                        <input type="password" value={(settings.cloud.provider as { secretKey: string }).secretKey || ""} onChange={(e) => updateCloudProvider("secretKey", e.target.value)} />
                      </div>
                    </div>
                  ) : (
                    <div className="proxy-settings-grid" style={{ gridTemplateColumns: "1fr 1fr", background: "none", padding: 0 }}>
                      <div className="form-group">
                        <label>{t.settings.cloudEndpoint}</label>
                        <input type="text" value={(settings.cloud.provider as { endpoint: string }).endpoint || ""} onChange={(e) => updateCloudProvider("endpoint", e.target.value)} placeholder="http://192.168.1.100:5244/dav" />
                      </div>
                      <div className="form-group">
                        <label>{t.settings.cloudUsername}</label>
                        <input type="text" value={(settings.cloud.provider as { username: string }).username || ""} onChange={(e) => updateCloudProvider("username", e.target.value)} />
                      </div>
                      <div className="form-group">
                        <label>{t.settings.cloudPassword}</label>
                        <input type="password" value={(settings.cloud.provider as { password: string }).password || ""} onChange={(e) => updateCloudProvider("password", e.target.value)} />
                      </div>
                      <div className="form-group">
                        <label>{t.settings.cloudRoot}</label>
                        <input type="text" value={(settings.cloud.provider as { root: string }).root || ""} onChange={(e) => updateCloudProvider("root", e.target.value)} placeholder="/DIT_Pro" />
                      </div>
                    </div>
                  )}

                  <div className="form-group" style={{ marginTop: 12 }}>
                    <button className="btn-secondary" onClick={testCloudConnection} disabled={testingCloud}>
                      {testingCloud ? "Testing..." : t.settings.cloudTestConnection}
                    </button>
                    {cloudTestResult && (
                      <span style={{ marginLeft: 12, fontSize: 12, color: cloudTestResult.success ? "#22c55e" : "#ef4444" }}>
                        {cloudTestResult.message}
                      </span>
                    )}
                  </div>
                </>
              )}
            </div>
          </div>
        </section>

        {/* ─── Diagnostics ──────────────────────────────────────── */}
        <section className="settings-section">
          <h3 className="section-title">{t.settings.diagnosticsTitle}</h3>
          <p className="section-desc">{t.settings.diagnosticsDesc}</p>
          <button
            className="btn-secondary"
            disabled={exportingBundle}
            onClick={async () => {
              setExportingBundle(true);
              setBundlePath(null);
              try {
                const result = await safeInvoke<CommandResult<string>>("export_debug_bundle");
                if (result.success && result.data) {
                  setBundlePath(result.data);
                } else {
                  setError(result.error || "Export failed");
                }
              } catch (err) {
                setError(String(err));
              }
              setExportingBundle(false);
            }}
          >
            {exportingBundle ? t.settings.exportingBundle : t.settings.exportDebugBundle}
          </button>
          {bundlePath && (
            <p className="bundle-path" style={{ marginTop: 8, fontSize: 13, color: "#9ca3af" }}>
              {t.settings.bundleSavedTo} <code>{bundlePath}</code>
            </p>
          )}
        </section>

        {/* ─── System Log ──────────────────────────────────────── */}
        <section className="settings-section">
          <h3 className="section-title">{t.systemLog.title}</h3>
          <p className="section-desc">{t.systemLog.desc}</p>
          <SystemLog />
        </section>
      </div>
    </div>
  );
}
