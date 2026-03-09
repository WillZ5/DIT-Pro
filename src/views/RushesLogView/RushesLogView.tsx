import { useState, useCallback } from "react";
import { safeInvoke, isTauri } from "../../utils/tauriCompat";
import { useI18n } from "../../i18n";
import type { CommandResult, RushesLogReport } from "../../types";

function formatBytes(bytes: number): string {
  if (bytes >= 1099511627776) return `${(bytes / 1099511627776).toFixed(1)} TB`;
  if (bytes >= 1073741824) return `${(bytes / 1073741824).toFixed(1)} GB`;
  if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${bytes} B`;
}

function formatDuration(seconds: number): string {
  const secs = Math.round(seconds);
  if (secs >= 3600) return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`;
  if (secs >= 60) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
  return `${secs}s`;
}

export function RushesLogView() {
  const { t } = useI18n();
  const [selectedDate, setSelectedDate] = useState<string>(
    new Date().toISOString().slice(0, 10)
  );
  const [report, setReport] = useState<RushesLogReport | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [successMsg, setSuccessMsg] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);

  const loadLog = useCallback(async () => {
    if (!selectedDate) return;
    setLoading(true);
    setError(null);
    setSuccessMsg(null);
    try {
      const result = await safeInvoke<CommandResult<RushesLogReport>>("get_rushes_log", {
        date: selectedDate,
      });
      if (result.success && result.data) {
        setReport(result.data);
      } else {
        setError(result.error || "Failed to load rushes log");
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, [selectedDate]);

  const handleExport = async (format: "csv" | "tsv" | "xlsx" | "pdf") => {
    if (!selectedDate) return;
    setExporting(true);
    setError(null);
    setSuccessMsg(null);
    try {
      let outputPath = "";
      if (isTauri()) {
        const { save } = await import("@tauri-apps/plugin-dialog");
        const ext = format;
        const filterName = format === "xlsx" ? "Excel" : format === "pdf" ? "PDF" : format.toUpperCase();
        const chosen = await save({
          title: t.rushesLog.exportCsv,
          defaultPath: `rushes-log-${selectedDate}.${ext}`,
          filters: [
            { name: filterName, extensions: [ext] },
          ],
        });
        if (!chosen) {
          setExporting(false);
          return;
        }
        outputPath = chosen;
      } else {
        outputPath = `/tmp/rushes-log-${selectedDate}.${format}`;
      }

      const result = await safeInvoke<CommandResult<string>>("export_rushes_log", {
        date: selectedDate,
        format,
        outputPath,
      });
      if (result.success && result.data) {
        setSuccessMsg(`${t.rushesLog.savedTo} ${result.data}`);
      } else {
        setError(result.error || "Export failed");
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setExporting(false);
    }
  };

  const handleCopyClipboard = async () => {
    if (!selectedDate) return;
    try {
      const result = await safeInvoke<CommandResult<string>>("copy_rushes_log_clipboard", {
        date: selectedDate,
      });
      if (result.success && result.data) {
        await navigator.clipboard.writeText(result.data);
        setSuccessMsg(t.rushesLog.copied);
        setTimeout(() => setSuccessMsg(null), 3000);
      } else {
        setError(result.error || "Copy failed");
      }
    } catch (err) {
      setError(String(err));
    }
  };

  const statusClass = (status: string) => {
    switch (status) {
      case "Verified": return "rushes-status-verified";
      case "Partial": return "rushes-status-partial";
      case "Failed": return "rushes-status-failed";
      default: return "rushes-status-pending";
    }
  };

  return (
    <div className="rushes-log-view">
      <div className="view-header">
        <h2>{t.rushesLog.title}</h2>
        <div className="rushes-log-toolbar">
          <input
            type="date"
            className="settings-input rushes-date-input"
            value={selectedDate}
            onChange={(e) => setSelectedDate(e.target.value)}
          />
          <button className="btn-primary" onClick={loadLog} disabled={loading}>
            {loading ? "..." : t.rushesLog.generateLog}
          </button>
        </div>
      </div>

      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button onClick={() => setError(null)}>&times;</button>
        </div>
      )}

      {successMsg && (
        <div className="success-banner">
          <span>{successMsg}</span>
          <button onClick={() => setSuccessMsg(null)}>&times;</button>
        </div>
      )}

      {!report && !loading && (
        <div className="empty-state">
          <h3>{t.rushesLog.noData}</h3>
          <p>{t.rushesLog.noDataHint}</p>
        </div>
      )}

      {report && report.entries.length === 0 && (
        <div className="empty-state">
          <h3>{t.rushesLog.noData}</h3>
          <p>{t.rushesLog.noDataHint}</p>
        </div>
      )}

      {report && report.entries.length > 0 && (
        <>
          {/* Summary cards */}
          <div className="rushes-log-summary">
            <div className="rushes-stat-card">
              <div className="rushes-stat-value">{report.summary.totalReels}</div>
              <div className="rushes-stat-label">{t.rushesLog.totalReels}</div>
            </div>
            <div className="rushes-stat-card">
              <div className="rushes-stat-value">{report.summary.totalClips}</div>
              <div className="rushes-stat-label">{t.rushesLog.totalClips}</div>
            </div>
            <div className="rushes-stat-card">
              <div className="rushes-stat-value">{formatBytes(report.summary.totalSize)}</div>
              <div className="rushes-stat-label">{t.rushesLog.totalSize}</div>
            </div>
            <div className="rushes-stat-card">
              <div className="rushes-stat-value">
                {report.summary.camerasUsed.length > 0
                  ? report.summary.camerasUsed.join(", ")
                  : "—"}
              </div>
              <div className="rushes-stat-label">{t.rushesLog.camerasUsed}</div>
            </div>
          </div>

          {/* Data table */}
          <div className="rushes-log-table-wrap">
            <table className="rushes-log-table">
              <thead>
                <tr>
                  <th>{t.rushesLog.reel}</th>
                  <th>{t.rushesLog.camera}</th>
                  <th>{t.rushesLog.clips}</th>
                  <th>{t.rushesLog.firstClip}</th>
                  <th>{t.rushesLog.lastClip}</th>
                  <th>{t.rushesLog.size}</th>
                  <th>{t.rushesLog.duration}</th>
                  <th>{t.rushesLog.speed}</th>
                  <th>{t.rushesLog.status}</th>
                  <th>{t.rushesLog.mhlVerified}</th>
                  <th>{t.rushesLog.resolution}</th>
                  <th>{t.rushesLog.frameRate}</th>
                  <th>{t.rushesLog.codec}</th>
                  <th>{t.rushesLog.destinations}</th>
                </tr>
              </thead>
              <tbody>
                {report.entries.map((entry) => (
                  <tr key={entry.jobId}>
                    <td className="rushes-cell-reel" title={entry.sourcePath}>{entry.reelName}</td>
                    <td>{entry.cameraBrand}{entry.cameraModel ? ` ${entry.cameraModel}` : ""}</td>
                    <td className="rushes-cell-num">{entry.clipCount}</td>
                    <td className="rushes-cell-clip" title={entry.firstClip}>{entry.firstClip}</td>
                    <td className="rushes-cell-clip" title={entry.lastClip}>{entry.lastClip}</td>
                    <td className="rushes-cell-num">{formatBytes(entry.totalSize)}</td>
                    <td className="rushes-cell-num">{formatDuration(entry.durationSeconds)}</td>
                    <td className="rushes-cell-num">{entry.avgSpeedMbps.toFixed(1)} MB/s</td>
                    <td>
                      <span className={`rushes-status-badge ${statusClass(entry.backupStatus)}`}>
                        {entry.backupStatus}
                      </span>
                    </td>
                    <td>
                      <span className={entry.mhlVerified ? "rushes-mhl-yes" : "rushes-mhl-no"}>
                        {entry.mhlVerified ? "✓" : "✗"}
                      </span>
                    </td>
                    <td className="rushes-cell-meta">{entry.resolution || "—"}</td>
                    <td className="rushes-cell-meta">{entry.frameRate || "—"}</td>
                    <td className="rushes-cell-meta">{entry.codec || "—"}</td>
                    <td className="rushes-cell-dest" title={entry.destPaths.join("\n")}>
                      {entry.destPaths.map((d) => d.split("/").pop()).join(", ")}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          {/* Export actions */}
          <div className="rushes-log-actions">
            <button
              className="btn-primary"
              onClick={handleCopyClipboard}
              title="Copy TSV to clipboard"
            >
              {t.rushesLog.copyClipboard}
            </button>
            <button
              className="btn-secondary"
              onClick={() => handleExport("csv")}
              disabled={exporting}
            >
              {exporting ? t.rushesLog.exporting : t.rushesLog.exportCsv}
            </button>
            <button
              className="btn-secondary"
              onClick={() => handleExport("tsv")}
              disabled={exporting}
            >
              {t.rushesLog.exportTsv}
            </button>
            <button
              className="btn-secondary"
              onClick={() => handleExport("xlsx")}
              disabled={exporting}
            >
              {t.rushesLog.exportXlsx}
            </button>
            <button
              className="btn-secondary"
              onClick={() => handleExport("pdf")}
              disabled={exporting}
            >
              {t.rushesLog.exportPdf}
            </button>
          </div>
        </>
      )}
    </div>
  );
}
