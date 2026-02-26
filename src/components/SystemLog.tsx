import { useState, useEffect, useCallback } from "react";
import { safeInvoke } from "../utils/tauriCompat";
import { useI18n } from "../i18n";
import type {
  CommandResult,
  ErrorLogEntry,
  ErrorLogSummary,
  ErrorLogFilter,
} from "../types";

// ─── Severity badge colors ────────────────────────────────────────────────

const SEVERITY_COLORS: Record<string, string> = {
  critical: "#ef4444",
  error: "#f97316",
  warning: "#eab308",
  info: "#3b82f6",
};

function SeverityBadge({ severity, label }: { severity: string; label: string }) {
  const bg = SEVERITY_COLORS[severity] || "#6b7280";
  return (
    <span
      style={{
        background: bg,
        color: "#fff",
        padding: "2px 8px",
        borderRadius: "4px",
        fontSize: "11px",
        fontWeight: 600,
        textTransform: "uppercase",
        letterSpacing: "0.5px",
      }}
    >
      {label}
    </span>
  );
}

// ─── Main Component ───────────────────────────────────────────────────────

export function SystemLog() {
  const { t } = useI18n();
  const [entries, setEntries] = useState<ErrorLogEntry[]>([]);
  const [summary, setSummary] = useState<ErrorLogSummary | null>(null);
  const [filterSeverity, setFilterSeverity] = useState<string>("");
  const [filterCategory, setFilterCategory] = useState<string>("");
  const [filterResolved, setFilterResolved] = useState<string>("");
  const [loading, setLoading] = useState(false);

  const severityLabel = (s: string) => {
    switch (s) {
      case "critical": return t.systemLog.severityCritical;
      case "error": return t.systemLog.severityError;
      case "warning": return t.systemLog.severityWarning;
      case "info": return t.systemLog.severityInfo;
      default: return s;
    }
  };

  const loadEntries = useCallback(async () => {
    setLoading(true);
    try {
      const filter: ErrorLogFilter = { limit: 200 };
      if (filterSeverity) filter.severity = filterSeverity;
      if (filterCategory) filter.category = filterCategory;
      if (filterResolved === "true") filter.resolved = true;
      if (filterResolved === "false") filter.resolved = false;

      const result = await safeInvoke<CommandResult<ErrorLogEntry[]>>(
        "get_error_log",
        { filter }
      );
      if (result.success && result.data) {
        setEntries(result.data);
      }
    } catch (err) {
      console.error("Failed to load error log:", err);
    }
    setLoading(false);
  }, [filterSeverity, filterCategory, filterResolved]);

  const loadSummary = useCallback(async () => {
    try {
      const result = await safeInvoke<CommandResult<ErrorLogSummary>>(
        "get_error_log_summary"
      );
      if (result.success && result.data) {
        setSummary(result.data);
      }
    } catch (err) {
      console.error("Failed to load error summary:", err);
    }
  }, []);

  useEffect(() => {
    loadEntries();
    loadSummary();
  }, [loadEntries, loadSummary]);

  const handleResolve = async (id: number) => {
    await safeInvoke<CommandResult<boolean>>("resolve_error_entry", { id });
    loadEntries();
    loadSummary();
  };

  const handleClearAll = async () => {
    if (!confirm(t.systemLog.clearConfirm)) return;
    await safeInvoke<CommandResult<number>>("clear_error_log_entries", {
      olderThanDays: null,
    });
    loadEntries();
    loadSummary();
  };

  const formatTime = (ts: string) => {
    try {
      const d = new Date(ts + "Z");
      return d.toLocaleString();
    } catch {
      return ts;
    }
  };

  const categories = ["COPY", "HASH", "DB", "CONFIG", "IO", "MHL", "NETWORK", "SYSTEM"];

  return (
    <div className="system-log">
      {/* Summary badges */}
      {summary && summary.total > 0 && (
        <div className="system-log-summary">
          <span className="summary-item">
            {t.systemLog.total}: <strong>{summary.total}</strong>
          </span>
          {summary.critical > 0 && (
            <span className="summary-item">
              <SeverityBadge severity="critical" label={`${t.systemLog.severityCritical}: ${summary.critical}`} />
            </span>
          )}
          {summary.error > 0 && (
            <span className="summary-item">
              <SeverityBadge severity="error" label={`${t.systemLog.severityError}: ${summary.error}`} />
            </span>
          )}
          {summary.warning > 0 && (
            <span className="summary-item">
              <SeverityBadge severity="warning" label={`${t.systemLog.severityWarning}: ${summary.warning}`} />
            </span>
          )}
          <span className="summary-item" style={{ marginLeft: "auto" }}>
            {t.systemLog.unresolved}: <strong>{summary.unresolved}</strong>
          </span>
        </div>
      )}

      {/* Filters */}
      <div className="system-log-filters">
        <select
          value={filterSeverity}
          onChange={(e) => setFilterSeverity(e.target.value)}
          className="filter-select"
        >
          <option value="">{t.systemLog.allSeverities}</option>
          <option value="critical">{t.systemLog.severityCritical}</option>
          <option value="error">{t.systemLog.severityError}</option>
          <option value="warning">{t.systemLog.severityWarning}</option>
          <option value="info">{t.systemLog.severityInfo}</option>
        </select>

        <select
          value={filterCategory}
          onChange={(e) => setFilterCategory(e.target.value)}
          className="filter-select"
        >
          <option value="">{t.systemLog.allCategories}</option>
          {categories.map((c) => (
            <option key={c} value={c}>{c}</option>
          ))}
        </select>

        <select
          value={filterResolved}
          onChange={(e) => setFilterResolved(e.target.value)}
          className="filter-select"
        >
          <option value="">{t.systemLog.resolved}/{t.systemLog.unresolved}</option>
          <option value="false">{t.systemLog.unresolved}</option>
          <option value="true">{t.systemLog.resolved}</option>
        </select>

        <button className="btn-sm btn-danger" onClick={handleClearAll}>
          {t.systemLog.clearAll}
        </button>
      </div>

      {/* Log entries */}
      {loading ? (
        <div className="system-log-loading">{t.common.loading}</div>
      ) : entries.length === 0 ? (
        <div className="system-log-empty">
          <p>{t.systemLog.noErrors}</p>
          <p className="hint">{t.systemLog.noErrorsHint}</p>
        </div>
      ) : (
        <div className="system-log-table-wrap">
          <table className="system-log-table">
            <thead>
              <tr>
                <th>{t.systemLog.timestamp}</th>
                <th>{t.systemLog.code}</th>
                <th>{t.systemLog.severity}</th>
                <th>{t.systemLog.category}</th>
                <th>{t.systemLog.message}</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {entries.map((entry) => (
                <tr key={entry.id} className={entry.resolved ? "resolved-row" : ""}>
                  <td className="ts-cell">{formatTime(entry.timestamp)}</td>
                  <td><code className="error-code">{entry.errorCode}</code></td>
                  <td>
                    <SeverityBadge severity={entry.severity} label={severityLabel(entry.severity)} />
                  </td>
                  <td className="cat-cell">{entry.category}</td>
                  <td className="msg-cell" title={entry.message}>
                    {entry.message.length > 120
                      ? entry.message.slice(0, 120) + "..."
                      : entry.message}
                    {entry.jobId && (
                      <span className="job-tag">{entry.jobId.slice(0, 8)}</span>
                    )}
                  </td>
                  <td>
                    {!entry.resolved && (
                      <button
                        className="btn-xs"
                        onClick={() => handleResolve(entry.id)}
                        title={t.systemLog.resolve}
                      >
                        {t.systemLog.resolve}
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
