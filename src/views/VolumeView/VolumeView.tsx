import { useState } from "react";
import type { VolumeInfo } from "../../types";

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

export function VolumeView() {
  const [volumes] = useState<VolumeInfo[]>([]);

  return (
    <div className="volume-view">
      <div className="view-header">
        <h2>Volumes</h2>
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
            <div key={vol.id} className={`volume-card ${vol.isMounted ? "mounted" : "unmounted"}`}>
              <div className="volume-name">{vol.name}</div>
              <div className="volume-type">{vol.deviceType}</div>
              <div className="volume-space">
                <span>{formatBytes(vol.availableBytes)} free</span>
                <span> / {formatBytes(vol.totalBytes)}</span>
              </div>
              <div className="volume-bar">
                <div
                  className="usage-bar"
                  style={{
                    width: `${((vol.totalBytes - vol.availableBytes) / vol.totalBytes) * 100}%`,
                  }}
                />
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
