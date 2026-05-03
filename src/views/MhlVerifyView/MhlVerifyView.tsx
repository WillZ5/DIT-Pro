import { useMemo, useState } from "react";
import { safeInvoke, isTauri } from "../../utils/tauriCompat";
import { useI18n } from "../../i18n";
import type { CommandResult, MhlVerifyOptions, MhlVerifyReport } from "../../types";

function formatDuration(seconds: number): string {
  if (!seconds || seconds < 0.01) return "< 0.01s";
  if (seconds < 1) return `${seconds.toFixed(2)}s`;
  return `${seconds.toFixed(1)}s`;
}

function basename(path: string): string {
  return path.split(/[/\\]/).filter(Boolean).pop() || path;
}

export function MhlVerifyView() {
  const { t } = useI18n();
  const [path, setPath] = useState("");
  const [chainOnly, setChainOnly] = useState(false);
  const [verifyAllGenerations, setVerifyAllGenerations] = useState(false);
  const [generation, setGeneration] = useState("");
  const [verifying, setVerifying] = useState(false);
  const [report, setReport] = useState<MhlVerifyReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  const canVerify = path.trim().length > 0 && !verifying;

  const statusLabel = useMemo(() => {
    if (!report) return t.mhlVerify.resultReady;
    return report.summary.success ? t.mhlVerify.resultPass : t.mhlVerify.resultFail;
  }, [report, t.mhlVerify.resultFail, t.mhlVerify.resultPass, t.mhlVerify.resultReady]);

  const handleSelectFolder = async () => {
    if (isTauri()) {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, title: t.mhlVerify.selectFolder });
      if (selected) setPath(selected as string);
    } else {
      setPath("/Volumes/RAID_SHUTTLE_01");
    }
  };

  const handleSelectManifest = async () => {
    if (isTauri()) {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        title: t.mhlVerify.selectManifest,
        filters: [{ name: "ASC MHL", extensions: ["mhl"] }],
      });
      if (selected) setPath(selected as string);
    } else {
      setPath("/Volumes/RAID_SHUTTLE_01/ascmhl/0001_Demo_2026-03-28_120000Z.mhl");
    }
  };

  const handleVerify = async () => {
    if (!canVerify) return;
    setVerifying(true);
    setError(null);
    setReport(null);

    const parsedGeneration = generation.trim() === "" ? null : Number(generation);
    const options: MhlVerifyOptions = {
      chainOnly,
      verifyAllGenerations,
      generation: Number.isFinite(parsedGeneration) && parsedGeneration ? parsedGeneration : null,
    };

    try {
      const result = await safeInvoke<CommandResult<MhlVerifyReport>>("verify_mhl_path", {
        path: path.trim(),
        options,
      });
      if (result.success && result.data) {
        setReport(result.data);
      } else {
        setError(result.error || "MHL verification failed");
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setVerifying(false);
    }
  };

  return (
    <div className="settings-view mhl-verify-view">
      <div className="view-header">
        <div>
          <h2>{t.mhlVerify.title}</h2>
          <p className="view-subtitle">{t.mhlVerify.subtitle}</p>
        </div>
        <div className="settings-actions">
          <button className="btn-primary" onClick={handleVerify} disabled={!canVerify}>
            {verifying ? t.mhlVerify.verifying : t.mhlVerify.verify}
          </button>
        </div>
      </div>

      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button onClick={() => setError(null)}>{t.common.dismiss}</button>
        </div>
      )}

      <div className="settings-sections">
        <section className="settings-section">
          <h3>{t.mhlVerify.pathLabel}</h3>
          <div className="path-selector">
            <input
              type="text"
              value={path}
              onChange={(event) => setPath(event.target.value)}
              placeholder={t.mhlVerify.pathPlaceholder}
            />
            <button className="btn-secondary" onClick={handleSelectFolder}>
              {t.mhlVerify.selectFolder}
            </button>
            <button className="btn-secondary" onClick={handleSelectManifest}>
              {t.mhlVerify.selectManifest}
            </button>
          </div>

          <div className="mhl-options-grid">
            <label className="checkbox-label">
              <input
                type="checkbox"
                checked={chainOnly}
                onChange={(event) => setChainOnly(event.target.checked)}
              />
              <span>
                {t.mhlVerify.chainOnly}
                <small>{t.mhlVerify.chainOnlyDesc}</small>
              </span>
            </label>
            <label className="checkbox-label">
              <input
                type="checkbox"
                checked={verifyAllGenerations}
                onChange={(event) => setVerifyAllGenerations(event.target.checked)}
                disabled={chainOnly || generation.trim() !== ""}
              />
              <span>
                {t.mhlVerify.verifyAllGenerations}
                <small>{t.mhlVerify.verifyAllGenerationsDesc}</small>
              </span>
            </label>
            <label className="mhl-generation-input">
              <span>{t.mhlVerify.generation}</span>
              <input
                type="number"
                min="1"
                value={generation}
                onChange={(event) => setGeneration(event.target.value)}
                placeholder={t.mhlVerify.generationPlaceholder}
                disabled={chainOnly || verifyAllGenerations}
              />
            </label>
          </div>
        </section>

        <section className="settings-section">
          <h3>{t.mhlVerify.resultReady}</h3>
          <div className="mhl-summary-grid">
            <div className={`mhl-result-card ${report?.summary.success ? "mhl-result-card--pass" : report ? "mhl-result-card--fail" : ""}`}>
              <span>{statusLabel}</span>
              <strong>{report ? basename(report.summary.path) : "--"}</strong>
            </div>
            <div className="mhl-result-card">
              <span>{t.mhlVerify.chain}</span>
              <strong>
                {report
                  ? `${report.summary.chainValid}/${report.summary.chainEntries || report.summary.chainValid}`
                  : "--"}
              </strong>
              {report && <small>{report.summary.chainInvalid} {t.mhlVerify.invalid}</small>}
            </div>
            <div className="mhl-result-card">
              <span>{t.mhlVerify.files}</span>
              <strong>
                {report ? `${report.summary.passed}/${report.summary.totalFiles}` : "--"}
              </strong>
              {report && (
                <small>
                  {report.summary.failed} {t.mhlVerify.failed} · {report.summary.missing} {t.mhlVerify.missing}
                </small>
              )}
            </div>
            <div className="mhl-result-card">
              <span>{t.mhlVerify.issues}</span>
              <strong>{report ? report.issues.length : "--"}</strong>
              {report && <small>{formatDuration(report.summary.durationSecs)}</small>}
            </div>
          </div>

          {report && (
            <div className="mhl-meta-row">
              <span>{t.mhlVerify.selectedPath}: <code>{report.summary.path}</code></span>
              {report.summary.verifiedGenerations.length > 0 && (
                <span>
                  {t.mhlVerify.verifiedGenerations}: {report.summary.verifiedGenerations.join(", ")}
                </span>
              )}
            </div>
          )}
        </section>

        {report && report.chainResults.length > 0 && (
          <section className="settings-section">
            <h3>{t.mhlVerify.chainResults}</h3>
            <div className="report-table-wrap">
              <table className="report-table">
                <thead>
                  <tr>
                    <th>{t.mhlVerify.issueGeneration}</th>
                    <th>{t.mhlVerify.manifest}</th>
                    <th>{t.reports.colStatus}</th>
                  </tr>
                </thead>
                <tbody>
                  {report.chainResults.map((result) => (
                    <tr key={`${result.generation}-${result.manifestPath}`}>
                      <td>{result.generation}</td>
                      <td><code>{basename(result.manifestPath)}</code></td>
                      <td>
                        <span className={`report-status ${result.valid ? "report-status--ok" : "report-status--error"}`}>
                          {result.valid ? t.mhlVerify.valid : t.mhlVerify.invalid}
                        </span>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>
        )}

        {report && (
          <section className="settings-section">
            <h3>{t.mhlVerify.issuesTitle}</h3>
            {report.issues.length === 0 ? (
              <div className="empty-state empty-state--compact">
                <p>{t.mhlVerify.noIssues}</p>
              </div>
            ) : (
              <div className="report-table-wrap">
                <table className="report-table">
                  <thead>
                    <tr>
                      <th>{t.mhlVerify.issueKind}</th>
                      <th>{t.mhlVerify.issueGeneration}</th>
                      <th>{t.mhlVerify.issueFile}</th>
                      <th>{t.mhlVerify.issueAlgorithm}</th>
                      <th>{t.mhlVerify.issueMessage}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {report.issues.map((issue, index) => (
                      <tr key={`${issue.kind}-${issue.relPath || issue.manifestPath}-${index}`}>
                        <td>{issue.kind}</td>
                        <td>{issue.generation ?? "--"}</td>
                        <td><code>{issue.relPath || basename(issue.manifestPath || "")}</code></td>
                        <td>{issue.algorithm || "--"}</td>
                        <td>{issue.message}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </section>
        )}
      </div>
    </div>
  );
}
