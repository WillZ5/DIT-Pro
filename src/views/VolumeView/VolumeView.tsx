import { useState, useEffect, useCallback } from "react";
import { safeInvoke, isTauri } from "../../utils/tauriCompat";
import { useI18n } from "../../i18n";
import type { CommandResult, VolumeInfoResponse } from "../../types";

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

function DeviceIcon({ type }: { type: string }) {
  switch (type) {
    case "SSD":
      return (
        <svg width="20" height="20" viewBox="0 0 20 20" fill="none">
          <rect x="2" y="5" width="16" height="10" rx="2" stroke="#60a5fa" strokeWidth="1.4" />
          <path d="M6 8l2 2-2 2" stroke="#60a5fa" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
          <path d="M10 12h4" stroke="#60a5fa" strokeWidth="1.4" strokeLinecap="round" />
        </svg>
      );
    case "NVMe":
      return (
        <svg width="20" height="20" viewBox="0 0 20 20" fill="none">
          <rect x="3" y="7" width="14" height="6" rx="1.5" stroke="#a78bfa" strokeWidth="1.4" />
          <path d="M7 7V5.5a1 1 0 011-1h4a1 1 0 011 1V7" stroke="#a78bfa" strokeWidth="1.4" />
          <circle cx="7" cy="10" r="1" fill="#a78bfa" />
          <circle cx="10" cy="10" r="1" fill="#a78bfa" />
          <circle cx="13" cy="10" r="1" fill="#a78bfa" />
        </svg>
      );
    case "HDD":
      return (
        <svg width="20" height="20" viewBox="0 0 20 20" fill="none">
          <circle cx="10" cy="10" r="7" stroke="#888" strokeWidth="1.4" />
          <circle cx="10" cy="10" r="2.5" stroke="#888" strokeWidth="1.4" />
          <circle cx="10" cy="10" r="0.8" fill="#888" />
        </svg>
      );
    case "RAID":
      return (
        <svg width="20" height="20" viewBox="0 0 20 20" fill="none">
          <rect x="2" y="4" width="6" height="12" rx="1" stroke="#f59e0b" strokeWidth="1.3" />
          <rect x="10" y="4" width="6" height="12" rx="1" stroke="#f59e0b" strokeWidth="1.3" />
          <circle cx="5" cy="8" r="0.8" fill="#f59e0b" />
          <circle cx="13" cy="8" r="0.8" fill="#f59e0b" />
          <path d="M5 12h0M13 12h0" stroke="#f59e0b" strokeWidth="1.5" strokeLinecap="round" />
          <path d="M8 10h2" stroke="#f59e0b" strokeWidth="1" strokeDasharray="1 1" />
        </svg>
      );
    case "Network":
      return (
        <svg width="20" height="20" viewBox="0 0 20 20" fill="none">
          <circle cx="10" cy="10" r="7" stroke="#22d3ee" strokeWidth="1.4" />
          <ellipse cx="10" cy="10" rx="3" ry="7" stroke="#22d3ee" strokeWidth="1.2" />
          <path d="M3 10h14" stroke="#22d3ee" strokeWidth="1.2" />
        </svg>
      );
    default:
      return (
        <svg width="20" height="20" viewBox="0 0 20 20" fill="none">
          <rect x="3" y="5" width="14" height="10" rx="2" stroke="#888" strokeWidth="1.4" />
          <circle cx="15" cy="10" r="1" fill="#888" />
        </svg>
      );
  }
}

export function VolumeView() {
  const { t } = useI18n();
  const [volumes, setVolumes] = useState<VolumeInfoResponse[]>([]);
  const [loading, setLoading] = useState(true);

  const loadVolumes = useCallback(async () => {
    try {
      const result = await safeInvoke<CommandResult<VolumeInfoResponse[]>>("list_volumes");
      if (result.success && result.data) {
        setVolumes(result.data);
      }
    } catch (err) {
      console.error("Failed to load volumes:", err);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadVolumes();
    const interval = setInterval(loadVolumes, 5000);
    return () => clearInterval(interval);
  }, [loadVolumes]);

  const getUsageBarColor = (vol: VolumeInfoResponse): string => {
    if (vol.isCritical) return "#ef4444";
    if (vol.isLow) return "#f59e0b";
    if (vol.usagePercent > 80) return "#f59e0b";
    return "#22c55e";
  };

  const handleOpenInFinder = async (mountPoint: string) => {
    if (isTauri()) {
      try {
        const { Command } = await import("@tauri-apps/plugin-shell");
        await Command.create("open", [mountPoint]).execute();
      } catch (err) {
        console.error("Failed to open in Finder:", err);
      }
    }
  };

  return (
    <div className="volume-view">
      <div className="view-header">
        <h2>{t.volumes.title}</h2>
        <button className="btn-secondary" onClick={loadVolumes} disabled={loading}>
          {loading ? t.volumes.scanning : t.volumes.refresh}
        </button>
      </div>

      {volumes.length === 0 ? (
        <div className="empty-state">
          <div className="empty-icon-svg">
            <svg width="48" height="48" viewBox="0 0 48 48" fill="none">
              <rect x="6" y="12" width="36" height="24" rx="4" stroke="#333" strokeWidth="2" />
              <circle cx="36" cy="24" r="2" fill="#333" />
              <path d="M12 20h16" stroke="#333" strokeWidth="2" strokeLinecap="round" />
              <path d="M12 28h10" stroke="#333" strokeWidth="2" strokeLinecap="round" />
            </svg>
          </div>
          <h3>{t.volumes.noVolumes}</h3>
          <p>{t.volumes.noVolumesHint}</p>
        </div>
      ) : (
        <div className="volumes-grid">
          {volumes.map((vol) => (
            <div
              key={vol.id}
              className={`volume-card ${!vol.isMounted ? "unmounted" : ""} ${vol.isCritical ? "critical" : vol.isLow ? "low" : ""}`}
              onClick={() => handleOpenInFinder(vol.mountPoint)}
              style={{ cursor: "pointer" }}
              title={t.volumes.openInFinder}
            >
              <div className="volume-header">
                <span className="volume-icon">
                  <DeviceIcon type={vol.deviceType} />
                </span>
                <div className="volume-header-text">
                  <span className="volume-name">{vol.name}</span>
                  <span className="volume-type">{vol.deviceType}{!vol.isMounted && ` \u00B7 ${t.volumes.unmounted}`}</span>
                </div>
              </div>
              <div className="volume-mount" title={vol.mountPoint}>
                {vol.mountPoint}
              </div>
              <div className="volume-space">
                <span className="space-free">{formatBytes(vol.availableBytes)} {t.common.free}</span>
                <span className="space-total"> / {formatBytes(vol.totalBytes)}</span>
              </div>
              <div className="volume-bar">
                <div
                  className="usage-bar"
                  style={{
                    width: `${vol.usagePercent}%`,
                    backgroundColor: getUsageBarColor(vol),
                  }}
                />
              </div>
              <div className="volume-percent">
                <span>{vol.usagePercent.toFixed(0)}% {t.common.used}</span>
                {vol.isCritical && <span className="warning-badge">{t.volumes.critical}</span>}
                {vol.isLow && !vol.isCritical && <span className="warning-badge low">{t.volumes.low}</span>}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
