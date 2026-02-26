import { useState, useEffect, useCallback } from "react";
import { JobsView } from "./views/JobsView/JobsView";
import { VolumeView } from "./views/VolumeView/VolumeView";
import { ReportView } from "./views/ReportView/ReportView";
import { PresetsView } from "./views/PresetsView/PresetsView";
import { SettingsView } from "./views/SettingsView/SettingsView";
import { safeInvoke, IS_DEMO, isTauri } from "./utils/tauriCompat";
import { useI18n } from "./i18n";
import type { ViewType, VersionInfo } from "./types";
import "./App.css";

/** Global ref for active job count — updated by JobsView, read by App */
let _activeJobCount = 0;
let _activeJobCountListener: ((count: number) => void) | null = null;
export function setActiveJobCount(count: number) {
  _activeJobCount = count;
  _activeJobCountListener?.(count);
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
  const [versionInfo, setVersionInfo] = useState<VersionInfo | null>(null);
  const [showQuitHint, setShowQuitHint] = useState(false);
  const [showAbout, setShowAbout] = useState(false);
  const [activeJobCount, setActiveJobCountState] = useState(0);
  const { t } = useI18n();

  // Sync the global _activeJobCount into React state for reactive quit dialog
  useEffect(() => {
    _activeJobCountListener = (count) => setActiveJobCountState(count);
    return () => { _activeJobCountListener = null; };
  }, []);

  const navItems: { id: ViewType; label: string; Icon: React.FC<{ active: boolean }> }[] = [
    { id: "jobs", label: t.nav.jobs, Icon: IconJobs },
    { id: "volumes", label: t.nav.volumes, Icon: IconVolumes },
    { id: "presets", label: t.nav.presets, Icon: IconPresets },
    { id: "reports", label: t.nav.reports, Icon: IconReports },
    { id: "settings", label: t.nav.settings, Icon: IconSettings },
  ];

  useEffect(() => {
    safeInvoke<VersionInfo>("get_app_version").then(setVersionInfo).catch(console.error);
  }, []);

  // Force quit — exit the entire application process
  const handleForceQuit = useCallback(async () => {
    if (!isTauri()) return;
    const { exit } = await import("@tauri-apps/plugin-process");
    await exit(0);
  }, []);

  // Edge-style hold ⌘Q to quit — uses keydown/keyup for real hold detection
  useEffect(() => {
    if (!isTauri()) return;

    let holdTimer: ReturnType<typeof setTimeout> | null = null;
    let hideTimer: ReturnType<typeof setTimeout> | null = null;
    let isHolding = false;

    const HOLD_DURATION = 1000; // 1 second hold to quit

    const onKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && (e.key === "q" || e.key === "Q")) {
        e.preventDefault();
        if (isHolding) return; // key repeat — already tracking

        isHolding = true;
        setShowQuitHint(true);
        if (hideTimer) { clearTimeout(hideTimer); hideTimer = null; }

        // If held for HOLD_DURATION → quit
        holdTimer = setTimeout(() => {
          if (isHolding) handleForceQuit();
        }, HOLD_DURATION);
      }
    };

    const onKeyUp = (e: KeyboardEvent) => {
      if (e.key === "q" || e.key === "Q" || e.key === "Meta" || e.key === "Control") {
        if (!isHolding) return;
        isHolding = false;
        if (holdTimer) { clearTimeout(holdTimer); holdTimer = null; }

        // Released early — keep toast visible briefly, then hide
        hideTimer = setTimeout(() => setShowQuitHint(false), 2000);
      }
    };

    document.addEventListener("keydown", onKeyDown, true);
    document.addEventListener("keyup", onKeyUp, true);

    return () => {
      document.removeEventListener("keydown", onKeyDown, true);
      document.removeEventListener("keyup", onKeyUp, true);
      if (holdTimer) clearTimeout(holdTimer);
      if (hideTimer) clearTimeout(hideTimer);
    };
  }, [handleForceQuit]);

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
          {versionInfo && versionInfo.channel !== "stable" && versionInfo.channel !== "dev" && (
            <span className={`channel-badge channel-${versionInfo.channel}`}>
              {versionInfo.channel.toUpperCase()}
            </span>
          )}
          <span
            className="version"
            title={versionInfo?.fullString || ""}
            onClick={() => setShowAbout(true)}
            style={{ cursor: "pointer" }}
          >
            v{versionInfo?.version || "\u2014"}
          </span>
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

      {/* About dialog */}
      {showAbout && versionInfo && (
        <div className="dialog-overlay" onClick={() => setShowAbout(false)}>
          <div className="dialog dialog--sm" onClick={(e) => e.stopPropagation()}>
            <div className="dialog-header">
              <h3>{t.about.title}</h3>
            </div>
            <div className="dialog-body">
              <table className="about-table">
                <tbody>
                  <tr>
                    <td className="about-label">{t.about.version}</td>
                    <td className="about-value">{versionInfo.fullString}</td>
                  </tr>
                  <tr>
                    <td className="about-label">{t.about.channel}</td>
                    <td className="about-value">
                      <span className={`channel-badge channel-${versionInfo.channel}`}>
                        {versionInfo.channel.toUpperCase()}
                      </span>
                    </td>
                  </tr>
                  {versionInfo.gitHash && (
                    <tr>
                      <td className="about-label">{t.about.gitHash}</td>
                      <td className="about-value"><code>{versionInfo.gitHash}</code></td>
                    </tr>
                  )}
                  {versionInfo.buildTime && (
                    <tr>
                      <td className="about-label">{t.about.buildTime}</td>
                      <td className="about-value">{versionInfo.buildTime}</td>
                    </tr>
                  )}
                  <tr>
                    <td className="about-label">{t.about.system}</td>
                    <td className="about-value">{navigator.platform}</td>
                  </tr>
                </tbody>
              </table>
            </div>
            <div className="dialog-footer">
              <button className="btn-secondary" onClick={() => setShowAbout(false)}>
                {t.about.close}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Hold-to-quit hint toast — Edge-style Cmd+Q behavior */}
      {showQuitHint && (
        <div className="quit-hint-toast">
          <span className="quit-hint-kbd">⌘Q</span>
          <span>{activeJobCount > 0 ? t.app.quitHoldHintActive : t.app.quitHoldHint}</span>
        </div>
      )}
    </div>
  );
}

export default App;
