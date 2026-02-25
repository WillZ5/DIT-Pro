import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  CommandResult,
  JobInfo,
  OffloadEvent,
  OffloadPhase,
  WorkflowPreset,
  StartOffloadRequest,
} from "../../types";

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
const PHASE_INFO: Record<OffloadPhase, { label: string; color: string }> = {
  PreFlight: { label: "Pre-Flight", color: "#ff9800" },
  SourceVerify: { label: "Hashing Source", color: "#9c27b0" },
  Copying: { label: "Copying", color: "#2196f3" },
  Cascading: { label: "Cascading", color: "#ff5722" },
  Verifying: { label: "Verifying", color: "#00bcd4" },
  Sealing: { label: "MHL Sealing", color: "#3f51b5" },
  Complete: { label: "Complete", color: "#4caf50" },
  Failed: { label: "Failed", color: "#f44336" },
};

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
  const [name, setName] = useState(
    `Offload ${new Date().toLocaleString()}`
  );
  const [sourcePath, setSourcePath] = useState("");
  const [destPaths, setDestPaths] = useState<string[]>([]);
  const [presets, setPresets] = useState<WorkflowPreset[]>([]);
  const [selectedPresetId, setSelectedPresetId] = useState<string | null>(null);

  // Offload options (can be overridden by preset)
  const [hashAlgorithms, setHashAlgorithms] = useState<string[]>(["XXH64", "SHA256"]);
  const [sourceVerify, setSourceVerify] = useState(true);
  const [postVerify, setPostVerify] = useState(true);
  const [generateMhl, setGenerateMhl] = useState(true);
  const [cascade, setCascade] = useState(false);

  // Load presets
  useEffect(() => {
    invoke<CommandResult<WorkflowPreset[]>>("list_presets").then((result) => {
      if (result.success && result.data) {
        setPresets(result.data);
      }
    });
  }, []);

  const handleSelectSource = async () => {
    const path = await open({
      directory: true,
      title: "Select Source Card / Directory",
    });
    if (path) setSourcePath(path as string);
  };

  const handleAddDest = async () => {
    const path = await open({
      directory: true,
      title: "Select Destination Directory",
    });
    if (path && !destPaths.includes(path as string)) {
      setDestPaths([...destPaths, path as string]);
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
    // Optionally apply default destinations from preset
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
          <h3>New Offload</h3>
          <button className="dialog-close" onClick={onCancel}>
            &times;
          </button>
        </div>

        <div className="dialog-body">
          {/* Job Name */}
          <div className="form-group">
            <label>Job Name</label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Day 1 A-Cam"
            />
          </div>

          {/* Source */}
          <div className="form-group">
            <label>Source Card / Directory</label>
            <div className="path-selector">
              <input
                type="text"
                value={sourcePath}
                readOnly
                placeholder="Select source directory..."
              />
              <button className="btn-secondary" onClick={handleSelectSource}>
                Browse
              </button>
            </div>
          </div>

          {/* Destinations (multi) */}
          <div className="form-group">
            <label>
              Destinations{" "}
              <span className="label-hint">
                ({destPaths.length} selected)
              </span>
            </label>
            <div className="dest-list">
              {destPaths.map((path, i) => (
                <div key={path} className="dest-item">
                  <span className="dest-index">{i === 0 && cascade ? "Primary" : `Dest ${i + 1}`}</span>
                  <span className="dest-path" title={path}>{path}</span>
                  <button
                    className="btn-icon btn-remove"
                    onClick={() => handleRemoveDest(i)}
                    title="Remove"
                  >
                    &times;
                  </button>
                </div>
              ))}
              <button className="btn-secondary btn-add-dest" onClick={handleAddDest}>
                + Add Destination
              </button>
            </div>
          </div>

          {/* Preset selector */}
          <div className="form-group">
            <label>Workflow Preset</label>
            <select
              value={selectedPresetId || ""}
              onChange={(e) =>
                e.target.value
                  ? handleApplyPreset(e.target.value)
                  : setSelectedPresetId(null)
              }
            >
              <option value="">Custom Configuration</option>
              {presets.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          </div>

          {/* Options Grid */}
          <div className="form-group">
            <label>Options</label>
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
                Source Verify
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
                Post Verify
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
                Generate MHL
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
                Cascade Copy
              </label>
            </div>
          </div>

          {/* Hash Algorithms */}
          <div className="form-group">
            <label>Hash Algorithms</label>
            <div className="options-grid">
              {["XXH64", "XXH3", "SHA256", "MD5"].map((algo) => (
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
            <div className="info-banner">
              Cascade mode: Files copy to <strong>{destPaths[0]?.split("/").pop()}</strong> first
              (fastest), then cascade to {destPaths.length - 1} secondary
              destination(s). Source card is freed sooner.
            </div>
          )}
        </div>

        <div className="dialog-footer">
          <button className="btn-secondary" onClick={onCancel}>
            Cancel
          </button>
          <button
            className="btn-primary"
            onClick={handleStart}
            disabled={!canStart}
          >
            Start Offload
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── Main Jobs View ─────────────────────────────────────────────────────

export function JobsView() {
  const [jobs, setJobs] = useState<JobInfo[]>([]);
  const [activeOffloads, setActiveOffloads] = useState<
    Map<string, ActiveOffload>
  >(new Map());
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showNewDialog, setShowNewDialog] = useState(false);

  // Ref to track the latest active offload ID for event routing
  const latestJobIdRef = useRef<string | null>(null);

  const loadJobs = useCallback(async () => {
    try {
      const result = await invoke<CommandResult<JobInfo[]>>("list_jobs");
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

  // Listen for real-time offload events
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;

    const setup = async () => {
      unlisten = await listen<OffloadEvent>("offload-event", (event) => {
        const ev = event.payload;

        setActiveOffloads((prev) => {
          const jobId = latestJobIdRef.current;
          if (!jobId) return prev;

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
      const result = await invoke<CommandResult<string>>("start_offload", {
        request,
      });

      if (result.success && result.data) {
        const jobId = result.data;
        latestJobIdRef.current = jobId;

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
      const result = await invoke<CommandResult<JobInfo>>("recover_job", {
        jobId,
      });
      if (result.success) {
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
        return "#4caf50";
      case "copying":
      case "verifying":
        return "#2196f3";
      case "failed":
        return "#f44336";
      case "pending":
        return "#ff9800";
      default:
        return "#9e9e9e";
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
        <h2>Jobs</h2>
        <button
          className="btn-primary"
          onClick={() => setShowNewDialog(true)}
          disabled={loading}
        >
          {loading ? "Starting..." : "+ New Offload"}
        </button>
      </div>

      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button onClick={() => setError(null)}>Dismiss</button>
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
          <h3>No active jobs</h3>
          <p>Insert a card and click "New Offload" to start copying.</p>
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

            return (
              <div
                key={offload.jobId}
                className={`job-card ${isRunning ? "job-card--active" : ""} ${offload.phase === "Failed" ? "job-card--failed" : ""} ${offload.phase === "Complete" ? "job-card--complete" : ""}`}
              >
                <div className="job-info">
                  <div className="job-header">
                    <span className="job-name">{offload.name}</span>
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
                  </div>

                  <div className="job-details">
                    {offload.currentFile && (
                      <span className="job-current-file" title={offload.currentFile}>
                        {offload.currentFile}
                      </span>
                    )}
                    <span className="job-stats">
                      {offload.completedFiles}/{offload.totalFiles} files
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

                {/* Completion summary */}
                {offload.phase === "Complete" && (
                  <div className="job-complete-summary">
                    <span className="complete-check">&#x2713;</span>
                    {offload.totalFiles} files copied in{" "}
                    {formatDuration(offload.durationSecs)}
                    {offload.totalBytes > 0 && (
                      <> &mdash; {formatBytes(offload.totalBytes)} total</>
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
          {dbOnlyJobs.map((job) => (
            <div key={job.id} className="job-card">
              <div className="job-info">
                <div className="job-header">
                  <span className="job-name">{job.name}</span>
                  <span
                    className="job-status"
                    style={{ color: getStatusColor(job.status) }}
                  >
                    {job.status.toUpperCase()}
                  </span>
                </div>
                <div className="job-details">
                  <span className="job-source" title={job.sourcePath}>
                    {job.sourcePath}
                  </span>
                  <span className="job-stats">
                    {job.completedTasks}/{job.totalTasks} files
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
              <div className="job-actions">
                {job.failedTasks > 0 && (
                  <span className="failed-badge">{job.failedTasks} failed</span>
                )}
                {(job.status === "pending" || job.failedTasks > 0) && (
                  <button
                    className="btn-secondary btn-sm"
                    onClick={() => handleRecover(job.id)}
                  >
                    Recover
                  </button>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
