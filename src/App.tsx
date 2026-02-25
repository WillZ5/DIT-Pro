import { useState, useEffect, useCallback } from "react";
import { JobsView } from "./views/JobsView/JobsView";
import { VolumeView } from "./views/VolumeView/VolumeView";
import { ReportView } from "./views/ReportView/ReportView";
import { PresetsView } from "./views/PresetsView/PresetsView";
import { SettingsView } from "./views/SettingsView/SettingsView";
import { safeInvoke, IS_DEMO, isTauri } from "./utils/tauriCompat";
import { useI18n } from "./i18n";
import type { ViewType } from "./types";
import "./App.css";

/** Global ref for active job count — updated by JobsView, read by App */
let _activeJobCount = 0;
export function setActiveJobCount(count: number) {
  _activeJobCount = count;
}
export function getActiveJobCount(): number {
  return _activeJobCount;
}

// ─── SVG Icon Components ──────────────────────────────────────────────────

function IconJobs({ active }: { active: boolean }) {
  return (
    <svg width="18" height="18" viewBox="0 0 18 18" fill="none">
      <rect x="2" y="3" width="14" height="12" rx="2" stroke={active ? "#60a5fa" : "currentColor"} strokeWidth="1.5" />
      <path d="M2 7h14" stroke={active ? "#60a5fa" : "currentColor"} strokeWidth="1.5" />
      <path d="M6 7V4" stroke={active ? "#60a5fa" : "currentColor"} strokeWidth="1.5" strokeLinecap="round" />
      <path d="M12 7V4" stroke={active ? "#60a5fa" : "currentColor"} strokeWidth="1.5" strokeLinecap="round" />
      <path d="M5.5 10.5h3" stroke={active ? "#60a5fa" : "currentColor"} strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  );
}

function IconVolumes({ active }: { active: boolean }) {
  return (
    <svg width="18" height="18" viewBox="0 0 18 18" fill="none">
      <rect x="2" y="4" width="14" height="10" rx="2" stroke={active ? "#60a5fa" : "currentColor"} strokeWidth="1.5" />
      <circle cx="13" cy="9" r="1" fill={active ? "#60a5fa" : "currentColor"} />
      <path d="M5 7h5" stroke={active ? "#60a5fa" : "currentColor"} strokeWidth="1.5" strokeLinecap="round" />
      <path d="M5 11h3" stroke={active ? "#60a5fa" : "currentColor"} strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  );
}

function IconPresets({ active }: { active: boolean }) {
  return (
    <svg width="18" height="18" viewBox="0 0 18 18" fill="none">
      <path d="M3 4.5h12M3 9h12M3 13.5h12" stroke={active ? "#60a5fa" : "currentColor"} strokeWidth="1.5" strokeLinecap="round" />
      <circle cx="6" cy="4.5" r="1.5" fill={active ? "#60a5fa" : "currentColor"} />
      <circle cx="12" cy="9" r="1.5" fill={active ? "#60a5fa" : "currentColor"} />
      <circle cx="8" cy="13.5" r="1.5" fill={active ? "#60a5fa" : "currentColor"} />
    </svg>
  );
}

function IconReports({ active }: { active: boolean }) {
  return (
    <svg width="18" height="18" viewBox="0 0 18 18" fill="none">
      <rect x="3" y="2" width="12" height="14" rx="1.5" stroke={active ? "#60a5fa" : "currentColor"} strokeWidth="1.5" />
      <path d="M6 6h6M6 9h6M6 12h3" stroke={active ? "#60a5fa" : "currentColor"} strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  );
}

function IconSettings({ active }: { active: boolean }) {
  return (
    <svg width="18" height="18" viewBox="0 0 18 18" fill="none">
      <circle cx="9" cy="9" r="2.5" stroke={active ? "#60a5fa" : "currentColor"} strokeWidth="1.5" />
      <path
        d="M9 1.5v2M9 14.5v2M1.5 9h2M14.5 9h2M3.7 3.7l1.4 1.4M12.9 12.9l1.4 1.4M3.7 14.3l1.4-1.4M12.9 5.1l1.4-1.4"
        stroke={active ? "#60a5fa" : "currentColor"}
        strokeWidth="1.3"
        strokeLinecap="round"
      />
    </svg>
  );
}

function App() {
  const [currentView, setCurrentView] = useState<ViewType>("jobs");
  const [appVersion, setAppVersion] = useState("");
  const [showQuitDialog, setShowQuitDialog] = useState(false);
  const { t } = useI18n();

  const navItems: { id: ViewType; label: string; Icon: React.FC<{ active: boolean }> }[] = [
    { id: "jobs", label: t.nav.jobs, Icon: IconJobs },
    { id: "volumes", label: t.nav.volumes, Icon: IconVolumes },
    { id: "presets", label: t.nav.presets, Icon: IconPresets },
    { id: "reports", label: t.nav.reports, Icon: IconReports },
    { id: "settings", label: t.nav.settings, Icon: IconSettings },
  ];

  useEffect(() => {
    safeInvoke<string>("get_app_version").then(setAppVersion).catch(console.error);
  }, []);

  // Force quit — exit the entire application process
  const handleForceQuit = useCallback(async () => {
    if (!isTauri()) return;
    const { exit } = await import("@tauri-apps/plugin-process");
    await exit(0);
  }, []);

  // Listen for quit-requested (Cmd+Q / Tray Quit) — always show confirmation
  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | null = null;
    const setup = async () => {
      const { listen } = await import("@tauri-apps/api/event");
      unlisten = await listen("quit-requested", () => {
        setShowQuitDialog(true);
      });
    };
    setup();
    return () => { unlisten?.(); };
  }, []);

  return (
    <div className="app">
      <nav className="sidebar">
        <div className="sidebar-header">
          <div className="brand">
            <div className="brand-icon">
              <svg width="22" height="22" viewBox="0 0 22 22" fill="none">
                <rect x="1" y="1" width="20" height="20" rx="4" stroke="#60a5fa" strokeWidth="1.5" />
                <path d="M6 7h10M6 11h10M6 15h6" stroke="#60a5fa" strokeWidth="1.5" strokeLinecap="round" />
                <circle cx="17" cy="15" r="2" fill="#60a5fa" />
              </svg>
            </div>
            <div className="brand-text">
              <h1 className="app-title">{t.app.title}</h1>
              <span className="brand-subtitle">{t.app.subtitle}</span>
            </div>
          </div>
        </div>

        <ul className="nav-list">
          {navItems.map((item) => {
            const isActive = currentView === item.id;
            return (
              <li key={item.id}>
                <button
                  className={`nav-item ${isActive ? "active" : ""}`}
                  onClick={() => setCurrentView(item.id)}
                >
                  <span className="nav-icon">
                    <item.Icon active={isActive} />
                  </span>
                  <span className="nav-label">{item.label}</span>
                </button>
              </li>
            );
          })}
        </ul>

        <div className="sidebar-footer">
          {IS_DEMO && <span className="demo-badge">{t.app.demo}</span>}
          <span className="version">v{appVersion || "\u2014"}</span>
        </div>
      </nav>

      <main className="main-content">
        {IS_DEMO && (
          <div className="demo-banner">
            {t.app.demoBanner}
          </div>
        )}
        {currentView === "jobs" && <JobsView />}
        {currentView === "volumes" && <VolumeView />}
        {currentView === "presets" && <PresetsView />}
        {currentView === "reports" && <ReportView />}
        {currentView === "settings" && <SettingsView />}
      </main>

      {/* Quit confirmation dialog — always shown on Cmd+Q / Tray Quit */}
      {showQuitDialog && (
        <div className="dialog-overlay" onClick={() => setShowQuitDialog(false)}>
          <div className="dialog dialog--sm" onClick={(e) => e.stopPropagation()}>
            <div className="dialog-header">
              <h3>{t.app.quitConfirmTitle}</h3>
            </div>
            <div className="dialog-body">
              <p>
                {_activeJobCount > 0
                  ? t.app.quitConfirmMessageActive
                  : t.app.quitConfirmMessage}
              </p>
            </div>
            <div className="dialog-footer">
              <button className="btn-secondary" onClick={() => setShowQuitDialog(false)}>
                {t.app.quitCancel}
              </button>
              <button
                className="btn-danger"
                onClick={() => {
                  setShowQuitDialog(false);
                  handleForceQuit();
                }}
              >
                {t.app.quitConfirm}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default App;
