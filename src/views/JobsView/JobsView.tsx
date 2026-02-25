import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { CommandResult, JobInfo } from "../../types";

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

export function JobsView() {
  const [jobs, setJobs] = useState<JobInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

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

  useEffect(() => {
    loadJobs();
    // Poll for updates every 2 seconds
    const interval = setInterval(loadJobs, 2000);
    return () => clearInterval(interval);
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
      const result = await invoke<CommandResult<JobInfo>>("create_job", {
        request: {
          name: `Offload ${new Date().toLocaleString()}`,
          sourcePath: sourcePath,
          destPaths: [destPath],
          hashAlgorithms: ["XXH64"],
        },
      });

      if (result.success) {
        await loadJobs();
      } else {
        setError(result.error || "Failed to create job");
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
      case "completed": return "#4caf50";
      case "copying":
      case "verifying": return "#2196f3";
      case "failed": return "#f44336";
      case "pending": return "#ff9800";
      default: return "#9e9e9e";
    }
  };

  return (
    <div className="jobs-view">
      <div className="view-header">
        <h2>Jobs</h2>
        <button
          className="btn-primary"
          onClick={handleNewOffload}
          disabled={loading}
        >
          {loading ? "Creating..." : "+ New Offload"}
        </button>
      </div>

      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button onClick={() => setError(null)}>Dismiss</button>
        </div>
      )}

      {jobs.length === 0 ? (
        <div className="empty-state">
          <div className="empty-icon">📁</div>
          <h3>No active jobs</h3>
          <p>Insert a card and click "New Offload" to start copying.</p>
        </div>
      ) : (
        <div className="jobs-list">
          {jobs.map((job) => (
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
                    {" · "}
                    {formatBytes(job.copiedBytes)} / {formatBytes(job.totalBytes)}
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
                  <span className="failed-badge">
                    {job.failedTasks} failed
                  </span>
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
