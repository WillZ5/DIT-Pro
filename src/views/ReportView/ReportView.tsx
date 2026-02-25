import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { CommandResult, DayReport, JobReport } from "../../types";

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

export function ReportView() {
  const [dates, setDates] = useState<string[]>([]);
  const [selectedDate, setSelectedDate] = useState<string | null>(null);
  const [dayReport, setDayReport] = useState<DayReport | null>(null);
  const [jobReport, setJobReport] = useState<JobReport | null>(null);
  const [exporting, setExporting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadDates = useCallback(async () => {
    try {
      const result = await invoke<CommandResult<string[]>>("get_report_dates");
      if (result.success && result.data) {
        setDates(result.data);
        if (result.data.length > 0 && !selectedDate) {
          setSelectedDate(result.data[0]);
        }
      }
    } catch (err) {
      console.error("Failed to load report dates:", err);
    }
  }, [selectedDate]);

  useEffect(() => {
    loadDates();
  }, [loadDates]);

  useEffect(() => {
    if (!selectedDate) return;
    (async () => {
      try {
        const result = await invoke<CommandResult<DayReport>>("get_day_report", {
          date: selectedDate,
        });
        if (result.success && result.data) {
          setDayReport(result.data);
          setJobReport(null);
        }
      } catch (err) {
        console.error("Failed to load day report:", err);
      }
    })();
  }, [selectedDate]);

  const handleViewJobDetail = async (jobId: string) => {
    try {
      const result = await invoke<CommandResult<JobReport>>("get_job_report", {
        jobId,
      });
      if (result.success && result.data) {
        setJobReport(result.data);
      }
    } catch (err) {
      setError(String(err));
    }
  };

  const handleExportDay = async () => {
    if (!selectedDate) return;
    setExporting(true);
    setError(null);
    try {
      const result = await invoke<CommandResult<string>>("export_day_report", {
        date: selectedDate,
      });
      if (result.success && result.data) {
        setError(null);
        alert(`Report saved to:\n${result.data}`);
      } else {
        setError(result.error || "Export failed");
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setExporting(false);
    }
  };

  const handleExportJob = async (jobId: string) => {
    setExporting(true);
    setError(null);
    try {
      const result = await invoke<CommandResult<string>>("export_job_report", {
        jobId,
      });
      if (result.success && result.data) {
        alert(`Report saved to:\n${result.data}`);
      } else {
        setError(result.error || "Export failed");
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setExporting(false);
    }
  };

  if (dates.length === 0) {
    return (
      <div className="settings-view">
        <div className="view-header">
          <h2>Reports</h2>
        </div>
        <div className="empty-state">
          <div className="empty-icon">📊</div>
          <h3>No reports yet</h3>
          <p>Reports will be generated after completing offload jobs.</p>
        </div>
      </div>
    );
  }

  return (
    <div className="settings-view">
      <div className="view-header">
        <h2>Reports</h2>
        <div className="settings-actions">
          <select
            className="settings-select"
            value={selectedDate || ""}
            onChange={(e) => setSelectedDate(e.target.value)}
          >
            {dates.map((d) => (
              <option key={d} value={d}>
                {d}
              </option>
            ))}
          </select>
          <button
            className="btn-primary"
            onClick={handleExportDay}
            disabled={exporting || !selectedDate}
          >
            {exporting ? "Exporting..." : "Export HTML"}
          </button>
        </div>
      </div>

      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button onClick={() => setError(null)}>Dismiss</button>
        </div>
      )}

      {dayReport && (
        <div className="settings-sections">
          {/* Day Summary Stats */}
          <section className="settings-section">
            <h3>Day Summary — {dayReport.date}</h3>
            <div className="report-stats">
              <div className="report-stat">
                <span className="report-stat-value">{dayReport.totalJobs}</span>
                <span className="report-stat-label">Total Jobs</span>
              </div>
              <div className="report-stat">
                <span className="report-stat-value">{dayReport.totalFiles}</span>
                <span className="report-stat-label">Total Files</span>
              </div>
              <div className="report-stat">
                <span className="report-stat-value">
                  {formatBytes(dayReport.totalBytes)}
                </span>
                <span className="report-stat-label">Total Data</span>
              </div>
              <div className="report-stat">
                <span className="report-stat-value">
                  {dayReport.completedJobs}/{dayReport.totalJobs}
                </span>
                <span className="report-stat-label">Completed</span>
              </div>
            </div>
          </section>

          {/* Job List */}
          <section className="settings-section">
            <h3>Jobs</h3>
            <div className="report-table-wrap">
              <table className="report-table">
                <thead>
                  <tr>
                    <th>Name</th>
                    <th>Status</th>
                    <th>Files</th>
                    <th>Size</th>
                    <th>Actions</th>
                  </tr>
                </thead>
                <tbody>
                  {dayReport.jobs.map((job) => (
                    <tr key={job.jobId}>
                      <td className="report-job-name">{job.jobName}</td>
                      <td>
                        <span
                          className={`report-status ${
                            job.status === "completed"
                              ? "report-status--ok"
                              : job.status.includes("error")
                                ? "report-status--error"
                                : "report-status--pending"
                          }`}
                        >
                          {job.status}
                        </span>
                      </td>
                      <td>
                        {job.completedFiles}/{job.totalFiles}
                      </td>
                      <td>{formatBytes(job.totalBytes)}</td>
                      <td>
                        <button
                          className="btn-small"
                          onClick={() => handleViewJobDetail(job.jobId)}
                        >
                          Detail
                        </button>
                        <button
                          className="btn-small"
                          onClick={() => handleExportJob(job.jobId)}
                          disabled={exporting}
                        >
                          Export
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>

          {/* Job Detail (when selected) */}
          {jobReport && (
            <section className="settings-section">
              <h3>
                Job Detail — {jobReport.summary.jobName}
                <button
                  className="btn-small"
                  style={{ marginLeft: 12 }}
                  onClick={() => setJobReport(null)}
                >
                  Close
                </button>
              </h3>
              <div className="report-stats">
                <div className="report-stat">
                  <span className="report-stat-value">
                    {jobReport.summary.totalFiles}
                  </span>
                  <span className="report-stat-label">Files</span>
                </div>
                <div className="report-stat">
                  <span className="report-stat-value">
                    {jobReport.summary.completedFiles}
                  </span>
                  <span className="report-stat-label">Completed</span>
                </div>
                <div className="report-stat">
                  <span className="report-stat-value">
                    {jobReport.summary.failedFiles}
                  </span>
                  <span className="report-stat-label">Failed</span>
                </div>
                <div className="report-stat">
                  <span className="report-stat-value">
                    {formatBytes(jobReport.summary.totalBytes)}
                  </span>
                  <span className="report-stat-label">Total</span>
                </div>
              </div>

              <div className="report-table-wrap">
                <table className="report-table report-table--detail">
                  <thead>
                    <tr>
                      <th>File</th>
                      <th>Destination</th>
                      <th>Size</th>
                      <th>Status</th>
                      <th>XXH64</th>
                      <th>SHA-256</th>
                    </tr>
                  </thead>
                  <tbody>
                    {jobReport.tasks.map((task, i) => {
                      const srcName =
                        task.sourcePath.split("/").pop() || task.sourcePath;
                      const destName =
                        task.destPath.split("/").pop() || task.destPath;
                      return (
                        <tr key={i}>
                          <td title={task.sourcePath}>{srcName}</td>
                          <td title={task.destPath}>{destName}</td>
                          <td>{formatBytes(task.fileSize)}</td>
                          <td>
                            <span
                              className={`report-status ${
                                task.status === "completed"
                                  ? "report-status--ok"
                                  : "report-status--error"
                              }`}
                            >
                              {task.status}
                            </span>
                          </td>
                          <td className="report-hash">{task.hashXxh64 || "—"}</td>
                          <td className="report-hash">
                            {task.hashSha256
                              ? task.hashSha256.slice(0, 16) + "..."
                              : "—"}
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            </section>
          )}
        </div>
      )}
    </div>
  );
}
