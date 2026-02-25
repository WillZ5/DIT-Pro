import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { CommandResult, VolumeInfoResponse } from "../../types";

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

export function VolumeView() {
  const [volumes, setVolumes] = useState<VolumeInfoResponse[]>([]);
  const [loading, setLoading] = useState(true);

  const loadVolumes = useCallback(async () => {
    try {
      const result = await invoke<CommandResult<VolumeInfoResponse[]>>("list_volumes");
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
    // Refresh every 5 seconds
    const interval = setInterval(loadVolumes, 5000);
    return () => clearInterval(interval);
  }, [loadVolumes]);

  const getUsageBarColor = (vol: VolumeInfoResponse): string => {
    if (vol.isCritical) return "#f44336";
    if (vol.isLow) return "#ff9800";
    if (vol.usagePercent > 80) return "#ff9800";
    return "#4caf50";
  };

  const getDeviceIcon = (deviceType: string): string => {
    switch (deviceType) {
      case "SSD": return "⚡";
      case "NVMe": return "🚀";
      case "HDD": return "💿";
      case "RAID": return "🏗️";
      case "Network": return "🌐";
      default: return "💾";
    }
  };

  return (
    <div className="volume-view">
      <div className="view-header">
        <h2>Volumes</h2>
        <button className="btn-secondary" onClick={loadVolumes} disabled={loading}>
          {loading ? "Scanning..." : "Refresh"}
        </button>
      </div>

      {volumes.length === 0 ? (
        <div className="empty-state">
          <div className="empty-icon">💾</div>
          <h3>No volumes detected</h3>
          <p>Connect an external drive to get started.</p>
        </div>
      ) : (
        <div className="volumes-grid">
          {volumes.map((vol) => (
            <div
              key={vol.id}
              className={`volume-card ${vol.isCritical ? "critical" : vol.isLow ? "low" : "healthy"}`}
            >
              <div className="volume-header">
                <span className="volume-icon">{getDeviceIcon(vol.deviceType)}</span>
                <span className="volume-name">{vol.name}</span>
              </div>
              <div className="volume-type">{vol.deviceType}</div>
              <div className="volume-mount" title={vol.mountPoint}>
                {vol.mountPoint}
              </div>
              <div className="volume-space">
                <span className="space-free">{formatBytes(vol.availableBytes)} free</span>
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
                {vol.usagePercent.toFixed(1)}% used
                {vol.isCritical && <span className="warning-badge">CRITICAL</span>}
                {vol.isLow && !vol.isCritical && <span className="warning-badge low">LOW</span>}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
