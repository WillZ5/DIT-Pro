import { useState, useEffect, useCallback } from "react";
import { safeInvoke } from "../../utils/tauriCompat";
import { useI18n, type TranslationKeys } from "../../i18n";
import type { CommandResult, DayReport, JobReport } from "../../types";

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

export function ReportView() {
  const { t } = useI18n();
  const [dates, setDates] = useState<string[]>([]);
  const [selectedDate, setSelectedDate] = useState<string | null>(null);
  const [dayReport, setDayReport] = useState<DayReport | null>(null);
  const [jobReport, setJobReport] = useState<JobReport | null>(null);
  const [exporting, setExporting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadDates = useCallback(async () => {
    try {
      const result = await safeInvoke<CommandResult<string[]>>("get_report_dates");
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
        const result = await safeInvoke<CommandResult<DayReport>>("get_day_report", {
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
      const result = await safeInvoke<CommandResult<JobReport>>("get_job_report", {
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
      const result = await safeInvoke<CommandResult<string>>("export_day_report", {
        date: selectedDate,
      });
      if (result.success && result.data) {
        setError(null);
        alert(`${t.reports.reportSavedTo}\n${result.data}`);
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
      const result = await safeInvoke<CommandResult<string>>("export_job_report", {
        jobId,
      });
      if (result.success && result.data) {
        alert(`${t.reports.reportSavedTo}\n${result.data}`);
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
          <h2>{t.reports.title}</h2>
        </div>
        <div className="empty-state">
          <div className="empty-icon-svg">
            <svg width="48" height="48" viewBox="0 0 48 48" fill="none">
              <rect x="10" y="6" width="28" height="36" rx="3" stroke="#333" strokeWidth="2" />
              <path d="M16 16h16M16 22h16M16 28h10" stroke="#333" strokeWidth="2" strokeLinecap="round" />
            </svg>
          </div>
          <h3>{t.reports.noReports}</h3>
          <p>{t.reports.noReportsHint}</p>
        </div>
      </div>
    );
  }

  return (
    <div className="settings-view">
      <div className="view-header">
        <h2>{t.reports.title}</h2>
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
            {exporting ? t.reports.exporting : t.reports.exportHtml}
          </button>
        </div>
      </div>

      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button onClick={() => setError(null)}>{t.common.dismiss}</button>
        </div>
      )}

      {dayReport && (
        <div className="settings-sections">
          {/* Day Summary Stats */}
          <section className="settings-section">
            <h3>{t.reports.daySummary} — {dayReport.date}</h3>
            <div className="report-stats">
              <div className="report-stat">
                <span className="report-stat-value">{dayReport.totalJobs}</span>
                <span className="report-stat-label">{t.reports.totalJobs}</span>
              </div>
              <div className="report-stat">
                <span className="report-stat-value">{dayReport.totalFiles}</span>
                <span className="report-stat-label">{t.reports.totalFiles}</span>
              </div>
              <div className="report-stat">
                <span className="report-stat-value">
                  {formatBytes(dayReport.totalBytes)}
                </span>
                <span className="report-stat-label">{t.reports.totalData}</span>
              </div>
              <div className="report-stat">
                <span className="report-stat-value">
                  {dayReport.completedJobs}/{dayReport.totalJobs}
                </span>
                <span className="report-stat-label">{t.reports.completed}</span>
              </div>
            </div>
          </section>

          {/* Job List */}
          <section className="settings-section">
            <h3>{t.reports.jobsTableTitle}</h3>
            <div className="report-table-wrap">
              <table className="report-table">
                <thead>
                  <tr>
                    <th>{t.reports.colName}</th>
                    <th>{t.reports.colStatus}</th>
                    <th>{t.reports.colFiles}</th>
                    <th>{t.reports.colSize}</th>
                    <th>{t.reports.colActions}</th>
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
                              : job.status === "completed_with_errors" || job.status === "failed" || job.status === "error"
                                ? "report-status--error"
                                : job.status === "copying" || job.status === "verifying"
                                  ? "report-status--active"
                                  : "report-status--pending"
                          }`}
                        >
                          {translateStatus(job.status, t)}
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
                          {t.reports.detail}
                        </button>
                        <button
                          className="btn-small"
                          onClick={() => handleExportJob(job.jobId)}
                          disabled={exporting}
                        >
                          {t.common.export}
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>

          {/* Job Detail */}
          {jobReport && (
            <section className="settings-section">
              <h3>
                {t.reports.jobDetail} — {jobReport.summary.jobName}
                <button
                  className="btn-small"
                  style={{ marginLeft: 12 }}
                  onClick={() => setJobReport(null)}
                >
                  {t.common.close}
                </button>
              </h3>
              <div className="report-stats">
                <div className="report-stat">
                  <span className="report-stat-value">
                    {jobReport.summary.totalFiles}
                  </span>
                  <span className="report-stat-label">{t.reports.colFiles}</span>
                </div>
                <div className="report-stat">
                  <span className="report-stat-value">
                    {jobReport.summary.completedFiles}
                  </span>
                  <span className="report-stat-label">{t.reports.completed}</span>
                </div>
                <div className="report-stat">
                  <span className="report-stat-value">
                    {jobReport.summary.failedFiles}
                  </span>
                  <span className="report-stat-label">{t.jobs.failed}</span>
                </div>
                <div className="report-stat">
                  <span className="report-stat-value">
                    {formatBytes(jobReport.summary.totalBytes)}
                  </span>
                  <span className="report-stat-label">{t.common.total}</span>
                </div>
              </div>

              <div className="report-table-wrap">
                <table className="report-table report-table--detail">
                  <thead>
                    <tr>
                      <th>{t.reports.colFile}</th>
                      <th>{t.reports.colDestination}</th>
                      <th>{t.reports.colSize}</th>
                      <th>{t.reports.colStatus}</th>
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
                                  : task.status === "pending"
                                    ? "report-status--pending"
                                    : task.status === "copying" || task.status === "verifying"
                                      ? "report-status--active"
                                      : "report-status--error"
                              }`}
                            >
                              {translateStatus(task.status, t)}
                            </span>
                          </td>
                          <td className="report-hash">{task.hashXxh64 || "\u2014"}</td>
                          <td className="report-hash">
                            {task.hashSha256
                              ? task.hashSha256.slice(0, 16) + "..."
                              : "\u2014"}
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
