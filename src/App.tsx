import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { JobsView } from "./views/JobsView/JobsView";
import { VolumeView } from "./views/VolumeView/VolumeView";
import { ReportView } from "./views/ReportView/ReportView";
import { PresetsView } from "./views/PresetsView/PresetsView";
import { SettingsView } from "./views/SettingsView/SettingsView";
import type { ViewType } from "./types";
import "./App.css";

function App() {
  const [currentView, setCurrentView] = useState<ViewType>("jobs");
  const [appVersion, setAppVersion] = useState("");

  useEffect(() => {
    invoke<string>("get_app_version").then(setAppVersion).catch(console.error);
  }, []);

  const navItems: { id: ViewType; label: string; icon: string }[] = [
    { id: "jobs", label: "Jobs", icon: "⚡" },
    { id: "volumes", label: "Volumes", icon: "💾" },
    { id: "presets", label: "Presets", icon: "📋" },
    { id: "reports", label: "Reports", icon: "📊" },
    { id: "settings", label: "Settings", icon: "⚙️" },
  ];

  return (
    <div className="app">
      <nav className="sidebar">
        <div className="sidebar-header">
          <h1 className="app-title">DIT System</h1>
        </div>
        <ul className="nav-list">
          {navItems.map((item) => (
            <li key={item.id}>
              <button
                className={`nav-item ${currentView === item.id ? "active" : ""}`}
                onClick={() => setCurrentView(item.id)}
              >
                <span className="nav-icon">{item.icon}</span>
                <span className="nav-label">{item.label}</span>
              </button>
            </li>
          ))}
        </ul>
        <div className="sidebar-footer">
          <span className="version">v{appVersion}</span>
        </div>
      </nav>

      <main className="main-content">
        {currentView === "jobs" && <JobsView />}
        {currentView === "volumes" && <VolumeView />}
        {currentView === "presets" && <PresetsView />}
        {currentView === "reports" && <ReportView />}
        {currentView === "settings" && <SettingsView />}
      </main>
    </div>
  );
}

export default App;
