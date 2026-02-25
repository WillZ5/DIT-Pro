import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  CommandResult,
  JobInfo,
  OffloadEvent,
  OffloadPhase,
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
  startedAt: number; // timestamp for speed calc
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

export function JobsView() {
  const [jobs, setJobs] = useState<JobInfo[]>([]);
  const [activeOffloads, setActiveOffloads] = useState<
    Map<string, ActiveOffload>
  >(new Map());
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

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
              // Reload DB jobs to reflect completed state
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

  const handleNewOffload = async () => {
    setError(null);
    try {
      // Select source directory
      const sourcePath = await open({
        directory: true,
        title: "Select Source Card / Directory",
      });
      if (!sourcePath) return;

      // Select destination directory
      const destPath = await open({
        directory: true,
        title: "Select Destination Directory",
      });
      if (!destPath) return;

      setLoading(true);
      const result = await invoke<CommandResult<string>>("start_offload", {
        request: {
          name: `Offload ${new Date().toLocaleString()}`,
          sourcePath: sourcePath,
          destPaths: [destPath],
          hashAlgorithms: ["XXH64"],
          sourceVerify: true,
          postVerify: true,
          generateMhl: true,
        },
      });

      if (result.success && result.data) {
        const jobId = result.data;
        const name = `Offload ${new Date().toLocaleString()}`;
        latestJobIdRef.current = jobId;

        setActiveOffloads((prev) => {
          const next = new Map(prev);
          next.set(jobId, createActiveOffload(jobId, name));
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
          onClick={handleNewOffload}
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
