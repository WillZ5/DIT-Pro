import { useState, useEffect, useCallback } from "react";
import { safeInvoke, isTauri } from "../../utils/tauriCompat";
import { setActiveJobCount } from "../../App";
import { useI18n, type TranslationKeys } from "../../i18n";
import type {
  AppSettings,
  CommandResult,
  JobInfo,
  OffloadEventEnvelope,
  OffloadPhase,
  WorkflowPreset,
  StartOffloadRequest,
} from "../../types";

/** Translate backend status string to localized display text */
function translateStatus(status: string, t: TranslationKeys): string {
  const map: Record<string, string> = {
    completed: t.jobs.statusCompleted,
    completed_with_errors: t.jobs.statusCompletedWithErrors,
    copying: t.jobs.statusCopying,
    verifying: t.jobs.statusVerifying,
    failed: t.jobs.statusFailed,
    pending: t.jobs.statusPending,
    error: t.jobs.statusError,
  };
  return map[status] || status.toUpperCase();
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

function formatSpeed(bytesPerSec: number): string {
  return `${formatBytes(bytesPerSec)}/s`;
}

function formatDuration(secs: number): string {
  if (secs < 60) return `${Math.round(secs)}s`;
  const m = Math.floor(secs / 60);
  const s = Math.round(secs % 60);
  return `${m}m ${s}s`;
}

/** Phase display configuration */
function usePhaseInfo(): Record<OffloadPhase, { label: string; color: string }> {
  const { t } = useI18n();
  return {
    PreFlight: { label: t.jobs.phasePreFlight, color: "#ff9800" },
    SourceVerify: { label: t.jobs.phaseSourceVerify, color: "#9c27b0" },
    Copying: { label: t.jobs.phaseCopying, color: "#2196f3" },
    Cascading: { label: t.jobs.phaseCascading, color: "#ff5722" },
    Verifying: { label: t.jobs.phaseVerifying, color: "#00bcd4" },
    Sealing: { label: t.jobs.phaseSealing, color: "#3f51b5" },
    Complete: { label: t.jobs.phaseComplete, color: "#4caf50" },
    Failed: { label: t.jobs.phaseFailed, color: "#f44336" },
  };
}

/** Live state for an active offload tracked via events */
interface ActiveOffload {
  jobId: string;
  name: string;
  phase: OffloadPhase;
  phaseMessage: string;
  currentFile: string;
  completedFiles: number;
  totalFiles: number;
  completedBytes: number;
  totalBytes: number;
  elapsedSecs: number;
  warnings: string[];
  error: string | null;
  mhlPaths: string[];
  durationSecs: number;
  startedAt: number;
  lastBytesSnapshot: number;
  lastSnapshotTime: number;
  currentSpeed: number;
}

function createActiveOffload(jobId: string, name: string): ActiveOffload {
  const now = Date.now();
  return {
    jobId,
    name,
    phase: "PreFlight",
    phaseMessage: "Starting...",
    currentFile: "",
    completedFiles: 0,
    totalFiles: 0,
    completedBytes: 0,
    totalBytes: 0,
    elapsedSecs: 0,
    warnings: [],
    error: null,
    mhlPaths: [],
    durationSecs: 0,
    startedAt: now,
    lastBytesSnapshot: 0,
    lastSnapshotTime: now,
    currentSpeed: 0,
  };
}

// ─── New Offload Dialog ─────────────────────────────────────────────────

interface NewOffloadDialogProps {
  onStart: (request: StartOffloadRequest) => void;
  onCancel: () => void;
}

function NewOffloadDialog({ onStart, onCancel }: NewOffloadDialogProps) {
  const { t } = useI18n();
  const [name, setName] = useState(
    `Offload ${new Date().toLocaleString()}`
  );
  const [sourcePath, setSourcePath] = useState("");
  const [destPaths, setDestPaths] = useState<string[]>([]);
  const [presets, setPresets] = useState<WorkflowPreset[]>([]);
  const [selectedPresetId, setSelectedPresetId] = useState<string | null>(null);

  // Offload options (can be overridden by preset or loaded from settings)
  const [hashAlgorithms, setHashAlgorithms] = useState<string[]>(["XXH64", "SHA256"]);
  const [sourceVerify, setSourceVerify] = useState(true);
  const [postVerify, setPostVerify] = useState(true);
  const [generateMhl, setGenerateMhl] = useState(true);
  const [cascade, setCascade] = useState(false);

  // Load presets + default settings
  useEffect(() => {
    safeInvoke<CommandResult<WorkflowPreset[]>>("list_presets").then((result) => {
      if (result.success && result.data) {
        setPresets(result.data);
      }
    });
    // Load default algorithms and offload options from settings
    safeInvoke<CommandResult<AppSettings>>("get_settings").then((result) => {
      if (result.success && result.data) {
        setHashAlgorithms(result.data.hashAlgorithms);
        setSourceVerify(result.data.offload.sourceVerify);
        setPostVerify(result.data.offload.postVerify);
        setGenerateMhl(result.data.offload.generateMhl);
        setCascade(result.data.offload.cascade);
      }
    });
  }, []);

  const handleSelectSource = async () => {
    if (isTauri()) {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const path = await open({ directory: true, title: t.jobs.sourceCard });
      if (path) setSourcePath(path as string);
    } else {
      setSourcePath("/Volumes/ALEXA_A001");
    }
  };

  const handleAddDest = async () => {
    if (isTauri()) {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const path = await open({ directory: true, title: t.jobs.destinations });
      if (path && !destPaths.includes(path as string)) {
        setDestPaths([...destPaths, path as string]);
      }
    } else {
      const demoPath = `/Volumes/RAID_SHUTTLE_0${destPaths.length + 1}`;
      if (!destPaths.includes(demoPath)) setDestPaths([...destPaths, demoPath]);
    }
  };

  const handleRemoveDest = (index: number) => {
    setDestPaths(destPaths.filter((_, i) => i !== index));
  };

  const handleApplyPreset = (presetId: string) => {
    const preset = presets.find((p) => p.id === presetId);
    if (!preset) {
      setSelectedPresetId(null);
      return;
    }
    setSelectedPresetId(presetId);
    setHashAlgorithms(preset.hashAlgorithms);
    setSourceVerify(preset.sourceVerify);
    setPostVerify(preset.postVerify);
    setGenerateMhl(preset.generateMhl);
    setCascade(preset.cascade);
    if (preset.defaultDestPaths.length > 0 && destPaths.length === 0) {
      setDestPaths(preset.defaultDestPaths);
    }
  };

  const toggleAlgorithm = (algo: string) => {
    if (hashAlgorithms.includes(algo)) {
      if (hashAlgorithms.length > 1) {
        setHashAlgorithms(hashAlgorithms.filter((a) => a !== algo));
      }
    } else {
      setHashAlgorithms([...hashAlgorithms, algo]);
    }
    setSelectedPresetId(null);
  };

  const canStart = sourcePath !== "" && destPaths.length > 0;

  const handleStart = () => {
    onStart({
      name,
      sourcePath,
      destPaths,
      hashAlgorithms,
      sourceVerify,
      postVerify,
      generateMhl,
      cascade,
    });
  };

  return (
    <div className="dialog-overlay" onClick={onCancel}>
      <div className="dialog" onClick={(e) => e.stopPropagation()}>
        <div className="dialog-header">
          <h3>{t.jobs.dialogTitle}</h3>
          <button className="dialog-close" onClick={onCancel}>
            &times;
          </button>
        </div>

        <div className="dialog-body">
          {/* Job Name */}
          <div className="form-group">
            <label>{t.jobs.jobName}</label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t.jobs.jobNamePlaceholder}
            />
          </div>

          {/* Source */}
          <div className="form-group">
            <label>{t.jobs.sourceCard}</label>
            <div className="path-selector">
              <input
                type="text"
                value={sourcePath}
                readOnly
                placeholder={t.jobs.sourcePlaceholder}
              />
              <button className="btn-secondary" onClick={handleSelectSource}>
                {t.common.browse}
              </button>
            </div>
          </div>

          {/* Destinations (multi) */}
          <div className="form-group">
            <label>
              {t.jobs.destinations}{" "}
              <span className="label-hint">
                ({destPaths.length} {t.jobs.selected})
              </span>
            </label>
            <div className="dest-list">
              {destPaths.map((path, i) => (
                <div key={path} className="dest-item">
                  <span className="dest-index">{i === 0 && cascade ? t.jobs.primary : `${t.jobs.dest} ${i + 1}`}</span>
                  <span className="dest-path" title={path}>{path}</span>
                  <button
                    className="btn-icon btn-remove"
                    onClick={() => handleRemoveDest(i)}
                    title={t.common.delete}
                  >
                    &times;
                  </button>
                </div>
              ))}
              <button className="btn-secondary btn-add-dest" onClick={handleAddDest}>
                {t.jobs.addDest}
              </button>
            </div>
          </div>

          {/* Preset selector */}
          <div className="form-group">
            <label>{t.jobs.workflowPreset}</label>
            <select
              value={selectedPresetId || ""}
              onChange={(e) =>
                e.target.value
                  ? handleApplyPreset(e.target.value)
                  : setSelectedPresetId(null)
              }
            >
              <option value="">{t.jobs.customConfig}</option>
              {presets.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          </div>

          {/* Options Grid */}
          <div className="form-group">
            <label>{t.jobs.options}</label>
            <div className="options-grid">
              <label className="checkbox-label">
                <input
                  type="checkbox"
                  checked={sourceVerify}
                  onChange={(e) => {
                    setSourceVerify(e.target.checked);
                    setSelectedPresetId(null);
                  }}
                />
                {t.jobs.sourceVerify}
              </label>
              <label className="checkbox-label">
                <input
                  type="checkbox"
                  checked={postVerify}
                  onChange={(e) => {
                    setPostVerify(e.target.checked);
                    setSelectedPresetId(null);
                  }}
                />
                {t.jobs.postVerify}
              </label>
              <label className="checkbox-label">
                <input
                  type="checkbox"
                  checked={generateMhl}
                  onChange={(e) => {
                    setGenerateMhl(e.target.checked);
                    setSelectedPresetId(null);
                  }}
                />
                {t.jobs.generateMhl}
              </label>
              <label className="checkbox-label">
                <input
                  type="checkbox"
                  checked={cascade}
                  onChange={(e) => {
                    setCascade(e.target.checked);
                    setSelectedPresetId(null);
                  }}
                />
                {t.jobs.cascadeCopy}
              </label>
            </div>
          </div>

          {/* Hash Algorithms */}
          <div className="form-group">
            <label>{t.jobs.hashAlgorithms}</label>
            <div className="options-grid">
              {["XXH64", "XXH3", "XXH128", "SHA256", "MD5"].map((algo) => (
                <label key={algo} className="checkbox-label">
                  <input
                    type="checkbox"
                    checked={hashAlgorithms.includes(algo)}
                    onChange={() => toggleAlgorithm(algo)}
                  />
                  {algo}
                </label>
              ))}
            </div>
          </div>

          {/* Cascade info */}
          {cascade && destPaths.length >= 2 && (
            <div
              className="info-banner"
              dangerouslySetInnerHTML={{
                __html: t.jobs.cascadeInfo
                  .replace("{dest}", destPaths[0]?.split("/").pop() || "")
                  .replace("{count}", String(destPaths.length - 1)),
              }}
            />
          )}
        </div>

        <div className="dialog-footer">
          <button className="btn-secondary" onClick={onCancel}>
            {t.common.cancel}
          </button>
          <button
            className="btn-primary"
            onClick={handleStart}
            disabled={!canStart}
          >
            {t.jobs.startOffload}
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── Main Jobs View ─────────────────────────────────────────────────────

export function JobsView() {
  const { t } = useI18n();
  const PHASE_INFO = usePhaseInfo();
  const [jobs, setJobs] = useState<JobInfo[]>([]);
  const [activeOffloads, setActiveOffloads] = useState<
    Map<string, ActiveOffload>
  >(new Map());
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showNewDialog, setShowNewDialog] = useState(false);
  const [expandedJobId, setExpandedJobId] = useState<string | null>(null);

  // Report active job count for exit confirmation dialog
  useEffect(() => {
    const running = Array.from(activeOffloads.values()).filter(
      (o) => o.phase !== "Complete" && o.phase !== "Failed"
    );
    setActiveJobCount(running.length);
    return () => setActiveJobCount(0);
  }, [activeOffloads]);

  const loadJobs = useCallback(async () => {
    try {
      const result = await safeInvoke<CommandResult<JobInfo[]>>("list_jobs");
      if (result.success && result.data) {
        setJobs(result.data);
      }
    } catch (err) {
      console.error("Failed to load jobs:", err);
    }
  }, []);

  // Poll DB jobs (for completed/historical jobs)
  useEffect(() => {
    loadJobs();
    const interval = setInterval(loadJobs, 3000);
    return () => clearInterval(interval);
  }, [loadJobs]);

  // Listen for real-time offload events (only in Tauri)
  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | null = null;

    const setup = async () => {
      const { listen } = await import("@tauri-apps/api/event");
      unlisten = await listen<OffloadEventEnvelope>("offload-event", (event) => {
        const { jobId, event: ev } = event.payload;

        setActiveOffloads((prev) => {
          const next = new Map(prev);
          const offload = next.get(jobId);
          if (!offload) return prev;

          const updated = { ...offload };
          const now = Date.now();

          switch (ev.type) {
            case "phaseChanged":
              updated.phase = ev.phase;
              updated.phaseMessage = ev.message;
              break;

            case "sourceHashCompleted":
              updated.currentFile = ev.relPath;
              updated.completedFiles = ev.fileIndex;
              updated.totalFiles = ev.totalFiles;
              break;

            case "fileCopyStarted":
              updated.currentFile = ev.relPath;
              break;

            case "fileCopyCompleted":
              updated.currentFile = ev.relPath;
              updated.completedFiles = ev.fileIndex;
              updated.totalFiles = ev.totalFiles;
              break;

            case "fileVerified":
              updated.currentFile = `Verify: ${ev.relPath}`;
              if (!ev.verified && ev.mismatchDetail) {
                updated.warnings.push(
                  `Verify failed: ${ev.relPath} - ${ev.mismatchDetail}`
                );
              }
              break;

            case "jobProgress": {
              updated.completedFiles = ev.completedFiles;
              updated.totalFiles = ev.totalFiles;
              updated.completedBytes = ev.completedBytes;
              updated.totalBytes = ev.totalBytes;
              updated.phase = ev.phase;
              updated.elapsedSecs = ev.elapsedSecs;
              // Calculate speed
              const dt = (now - updated.lastSnapshotTime) / 1000;
              if (dt > 0.5) {
                const db = ev.completedBytes - updated.lastBytesSnapshot;
                updated.currentSpeed = db / dt;
                updated.lastBytesSnapshot = ev.completedBytes;
                updated.lastSnapshotTime = now;
              }
              break;
            }

            case "warning":
              updated.warnings = [...updated.warnings, ev.message];
              break;

            case "complete":
              updated.phase = "Complete";
              updated.phaseMessage = "Offload complete";
              updated.completedFiles = ev.totalFiles;
              updated.totalFiles = ev.totalFiles;
              updated.completedBytes = ev.totalBytes;
              updated.totalBytes = ev.totalBytes;
              updated.durationSecs = ev.durationSecs;
              updated.mhlPaths = ev.mhlPaths;
              loadJobs();
              break;

            case "error":
              updated.phase = "Failed";
              updated.error = ev.message;
              updated.phaseMessage = ev.message;
              loadJobs();
              break;
          }

          next.set(jobId, updated);
          return next;
        });
      });
    };

    setup();
    return () => {
      unlisten?.();
    };
  }, [loadJobs]);

  const handleStartOffload = async (request: StartOffloadRequest) => {
    setShowNewDialog(false);
    setError(null);

    try {
      setLoading(true);
      const result = await safeInvoke<CommandResult<string>>("start_offload", {
        request,
      });

      if (result.success && result.data) {
        const jobId = result.data;

        setActiveOffloads((prev) => {
          const next = new Map(prev);
          next.set(jobId, createActiveOffload(jobId, request.name));
          return next;
        });
      } else {
        setError(result.error || "Failed to start offload");
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  };

  const handleRecover = async (jobId: string) => {
    try {
      setError(null);
      // Resume offload: recovers tasks + restarts the workflow
      const result = await safeInvoke<CommandResult<string>>("resume_offload", {
        jobId,
      });
      if (result.success && result.data) {
        // Track as active offload for real-time events
        const job = jobs.find((j) => j.id === jobId);
        setActiveOffloads((prev) => {
          const next = new Map(prev);
          next.set(jobId, createActiveOffload(jobId, job?.name || "Resumed Offload"));
          return next;
        });
        await loadJobs();
      } else {
        setError(result.error || "Recovery failed");
      }
    } catch (err) {
      setError(String(err));
    }
  };

  const getStatusColor = (status: string): string => {
    switch (status) {
      case "completed":
        return "#22c55e";
      case "copying":
      case "verifying":
        return "#3b82f6";
      case "failed":
      case "completed_with_errors":
        return "#ef4444";
      case "pending":
        return "#f59e0b";
      default:
        return "#71717a";
    }
  };

  const getStatusBadgeClass = (status: string): string => {
    switch (status) {
      case "completed":
        return "status-badge status-badge--completed";
      case "copying":
      case "verifying":
        return "status-badge status-badge--active";
      case "failed":
      case "completed_with_errors":
        return "status-badge status-badge--failed";
      case "pending":
        return "status-badge status-badge--pending";
      default:
        return "status-badge";
    }
  };

  // Combine active offloads (sorted newest first) + DB jobs
  const activeList = Array.from(activeOffloads.values()).sort(
    (a, b) => b.startedAt - a.startedAt
  );

  // Filter out DB jobs that already have an active offload tracker
  const activeJobIds = new Set(activeList.map((o) => o.jobId));
  const dbOnlyJobs = jobs.filter((j) => !activeJobIds.has(j.id));

  return (
    <div className="jobs-view">
      <div className="view-header">
        <h2>{t.jobs.title}</h2>
        <button
          className="btn-primary"
          onClick={() => setShowNewDialog(true)}
          disabled={loading}
        >
          {loading ? t.jobs.starting : t.jobs.newOffload}
        </button>
      </div>

      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button onClick={() => setError(null)}>{t.common.dismiss}</button>
        </div>
      )}

      {/* New Offload Dialog */}
      {showNewDialog && (
        <NewOffloadDialog
          onStart={handleStartOffload}
          onCancel={() => setShowNewDialog(false)}
        />
      )}

      {activeList.length === 0 && dbOnlyJobs.length === 0 ? (
        <div className="empty-state">
          <div className="empty-icon">&#x1F4C1;</div>
          <h3>{t.jobs.noJobs}</h3>
          <p>{t.jobs.noJobsHint}</p>
        </div>
      ) : (
        <div className="jobs-list">
          {/* Active offloads with live event data */}
          {activeList.map((offload) => {
            const phaseInfo = PHASE_INFO[offload.phase];
            const pct =
              offload.totalBytes > 0
                ? (offload.completedBytes / offload.totalBytes) * 100
                : offload.totalFiles > 0
                  ? (offload.completedFiles / offload.totalFiles) * 100
                  : 0;
            const isRunning =
              offload.phase !== "Complete" && offload.phase !== "Failed";
            const eta =
              isRunning && offload.currentSpeed > 0
                ? (offload.totalBytes - offload.completedBytes) /
                  offload.currentSpeed
                : 0;

            const isExpanded = expandedJobId === offload.jobId;

            return (
              <div
                key={offload.jobId}
                className={`job-card ${isRunning ? "job-card--active" : ""} ${offload.phase === "Failed" ? "job-card--failed" : ""} ${offload.phase === "Complete" ? "job-card--complete" : ""}`}
              >
                <div className="job-info">
                  <div className="job-header">
                    <span className="job-name">{offload.name}</span>
                    <div className="job-header-right">
                      <span
                        className="job-phase-badge"
                        style={{
                          backgroundColor: phaseInfo.color + "22",
                          color: phaseInfo.color,
                          borderColor: phaseInfo.color + "44",
                        }}
                      >
                        {isRunning && (
                          <span className="pulse-dot" style={{ backgroundColor: phaseInfo.color }} />
                        )}
                        {phaseInfo.label}
                      </span>
                      <button
                        className={`job-expand-btn ${isExpanded ? "job-expand-btn--open" : ""}`}
                        onClick={() => setExpandedJobId(isExpanded ? null : offload.jobId)}
                        title={t.jobs.details}
                      >
                        <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
                          <path d="M4 5.5L7 8.5L10 5.5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"/>
                        </svg>
                      </button>
                    </div>
                  </div>

                  <div className="job-details">
                    {offload.currentFile && (
                      <span className="job-current-file" title={offload.currentFile}>
                        {offload.currentFile}
                      </span>
                    )}
                    <span className="job-stats">
                      {offload.completedFiles}/{offload.totalFiles} {t.common.files}
                      {offload.totalBytes > 0 && (
                        <>
                          {" \u00B7 "}
                          {formatBytes(offload.completedBytes)} /{" "}
                          {formatBytes(offload.totalBytes)}
                        </>
                      )}
                      {isRunning && offload.currentSpeed > 0 && (
                        <>
                          {" \u00B7 "}
                          {formatSpeed(offload.currentSpeed)}
                        </>
                      )}
                      {isRunning && eta > 0 && (
                        <>
                          {" \u00B7 ETA "}
                          {formatDuration(eta)}
                        </>
                      )}
                      {offload.elapsedSecs > 0 && (
                        <>
                          {" \u00B7 "}
                          {formatDuration(offload.elapsedSecs)}
                        </>
                      )}
                    </span>
                  </div>
                </div>

                <div className="job-progress">
                  <div
                    className={`progress-bar ${isRunning ? "progress-bar--animated" : ""}`}
                    style={{
                      width: `${Math.min(pct, 100)}%`,
                      backgroundColor: phaseInfo.color,
                    }}
                  />
                </div>

                {/* Expandable detail panel */}
                {isExpanded && (
                  <div className="job-detail-panel">
                    <div className="job-detail-grid">
                      <div className="job-detail-item">
                        <span className="job-detail-label">{t.jobs.currentFile}</span>
                        <span className="job-detail-value job-detail-file" title={offload.currentFile}>
                          {offload.currentFile || "—"}
                        </span>
                      </div>
                      <div className="job-detail-item">
                        <span className="job-detail-label">{t.jobs.speed}</span>
                        <span className="job-detail-value">
                          {offload.currentSpeed > 0 ? formatSpeed(offload.currentSpeed) : "—"}
                        </span>
                      </div>
                      <div className="job-detail-item">
                        <span className="job-detail-label">{t.jobs.elapsed}</span>
                        <span className="job-detail-value">
                          {offload.elapsedSecs > 0 ? formatDuration(offload.elapsedSecs) : "—"}
                        </span>
                      </div>
                      <div className="job-detail-item">
                        <span className="job-detail-label">{t.jobs.eta}</span>
                        <span className="job-detail-value">
                          {isRunning && eta > 0 ? formatDuration(eta) : "—"}
                        </span>
                      </div>
                      <div className="job-detail-item">
                        <span className="job-detail-label">{t.reports.colFiles}</span>
                        <span className="job-detail-value">
                          {offload.completedFiles} / {offload.totalFiles}
                        </span>
                      </div>
                      <div className="job-detail-item">
                        <span className="job-detail-label">{t.reports.colSize}</span>
                        <span className="job-detail-value">
                          {formatBytes(offload.completedBytes)} / {formatBytes(offload.totalBytes)}
                        </span>
                      </div>
                    </div>
                    {offload.phaseMessage && (
                      <div className="job-detail-phase-msg">{offload.phaseMessage}</div>
                    )}
                  </div>
                )}

                {/* Completion summary */}
                {offload.phase === "Complete" && (
                  <div className="job-complete-summary">
                    <span className="complete-check">&#x2713;</span>
                    {offload.totalFiles} {t.jobs.filesCopiedIn}{" "}
                    {formatDuration(offload.durationSecs)}
                    {offload.totalBytes > 0 && (
                      <> &mdash; {formatBytes(offload.totalBytes)} {t.common.total}</>
                    )}
                    {offload.mhlPaths.length > 0 && (
                      <span className="mhl-badge">MHL</span>
                    )}
                  </div>
                )}

                {/* Error display */}
                {offload.error && (
                  <div className="job-error-msg">{offload.error}</div>
                )}

                {/* Warnings */}
                {offload.warnings.length > 0 && (
                  <div className="job-warnings">
                    {offload.warnings.map((w, i) => (
                      <div key={i} className="job-warning-item">
                        &#x26A0; {w}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            );
          })}

          {/* Historical / DB-only jobs */}
          {dbOnlyJobs.map((job) => {
            const isExpanded = expandedJobId === job.id;
            return (
              <div key={job.id} className="job-card">
                <div className="job-info">
                  <div className="job-header">
                    <span className="job-name">{job.name}</span>
                    <div className="job-header-right">
                      <span className={getStatusBadgeClass(job.status)}>
                        {translateStatus(job.status, t)}
                      </span>
                      <button
                        className={`job-expand-btn ${isExpanded ? "job-expand-btn--open" : ""}`}
                        onClick={() => setExpandedJobId(isExpanded ? null : job.id)}
                        title={t.jobs.details}
                      >
                        <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
                          <path d="M4 5.5L7 8.5L10 5.5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"/>
                        </svg>
                      </button>
                    </div>
                  </div>
                  <div className="job-details">
                    <span className="job-source" title={job.sourcePath}>
                      {job.sourcePath}
                    </span>
                    <span className="job-stats">
                      {job.completedTasks}/{job.totalTasks} {t.common.files}
                      {" \u00B7 "}
                      {formatBytes(job.copiedBytes)} /{" "}
                      {formatBytes(job.totalBytes)}
                    </span>
                  </div>
                </div>
                <div className="job-progress">
                  <div
                    className="progress-bar"
                    style={{
                      width: `${job.progressPercent}%`,
                      backgroundColor: getStatusColor(job.status),
                    }}
                  />
                </div>

                {/* Expandable detail panel */}
                {isExpanded && (
                  <div className="job-detail-panel">
                    <div className="job-detail-grid">
                      <div className="job-detail-item">
                        <span className="job-detail-label">{t.jobs.sourceCard}</span>
                        <span className="job-detail-value job-detail-file" title={job.sourcePath}>
                          {job.sourcePath}
                        </span>
                      </div>
                      <div className="job-detail-item">
                        <span className="job-detail-label">{t.reports.colStatus}</span>
                        <span className="job-detail-value">
                          {translateStatus(job.status, t)}
                        </span>
                      </div>
                      <div className="job-detail-item">
                        <span className="job-detail-label">{t.reports.colFiles}</span>
                        <span className="job-detail-value">
                          {job.completedTasks} / {job.totalTasks}
                        </span>
                      </div>
                      <div className="job-detail-item">
                        <span className="job-detail-label">{t.reports.colSize}</span>
                        <span className="job-detail-value">
                          {formatBytes(job.copiedBytes)} / {formatBytes(job.totalBytes)}
                        </span>
                      </div>
                      <div className="job-detail-item">
                        <span className="job-detail-label">{t.jobs.failed}</span>
                        <span className="job-detail-value" style={job.failedTasks > 0 ? { color: "#ef4444" } : undefined}>
                          {job.failedTasks}
                        </span>
                      </div>
                      <div className="job-detail-item">
                        <span className="job-detail-label">{t.common.total}</span>
                        <span className="job-detail-value">
                          {job.progressPercent.toFixed(1)}%
                        </span>
                      </div>
                    </div>
                  </div>
                )}

                <div className="job-actions">
                  {job.failedTasks > 0 && (
                    <span className="failed-badge">{job.failedTasks} {t.jobs.failed}</span>
                  )}
                  {(job.status === "pending" || job.status === "copying" || job.failedTasks > 0) && (
                    <button
                      className="btn-secondary btn-sm"
                      onClick={() => handleRecover(job.id)}
                    >
                      {t.common.recover}
                    </button>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
