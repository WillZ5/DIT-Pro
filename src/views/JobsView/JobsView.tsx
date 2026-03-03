import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { safeInvoke, isTauri } from "../../utils/tauriCompat";
import { setActiveJobCount } from "../../state/activeJobCount";
import { useI18n, type TranslationKeys } from "../../i18n";
import type {
  AppSettings,
  CommandResult,
  ConflictAction,
  ConflictResolution,
  FileConflict,
  JobInfo,
  OffloadEventEnvelope,
  OffloadPhase,
  VolumeInfoResponse,
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
    paused: t.jobs.statusPaused,
    terminated: t.jobs.statusTerminated,
    skipped: t.jobs.skippedFiles,
    conflict: t.jobs.conflicts,
  };
  return map[status] || status.toUpperCase();
}

function formatBytes(bytes: number): string {
  if (!bytes || bytes <= 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(k)), sizes.length - 1);
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

function formatSpeed(bytesPerSec: number): string {
  if (!bytesPerSec || bytesPerSec <= 0) return "0 B/s";
  return `${formatBytes(bytesPerSec)}/s`;
}

function formatDuration(secs: number): string {
  if (secs === undefined || secs === null || secs < 0) return "0s";
  if (secs > 0 && secs < 1) return "< 1s";
  const totalSecs = Math.max(1, Math.round(secs));
  if (totalSecs < 60) return `${totalSecs}s`;
  const m = Math.floor(totalSecs / 60);
  const s = totalSecs % 60;
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
    Terminated: { label: t.jobs.statusTerminated, color: "#dc2626" },
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
  isPaused: boolean;
  speedHistoryByPhase: Record<string, number[]>;
  /** Chronological speed timeline preserving phase ordering (handles Copying→Verify→Copying retry) */
  speedTimeline: Array<{ phase: string; speed: number }>;
  failedFiles: number;
  sourceReleased: boolean;
  sourceReleasedPath: string;
}

interface SpaceIssueInfo {
  path: string;
  availableBytes: number;
  requiredBytes: number;
  deficitBytes: number;
}

interface SourceSizeInfo {
  totalFiles: number;
  totalBytes: number;
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
    isPaused: false,
    speedHistoryByPhase: {},
    speedTimeline: [],
    failedFiles: 0,
    sourceReleased: false,
    sourceReleasedPath: "",
  };
}

// ─── Conflict Resolution Dialog ─────────────────────────────────────────

// ─── Space Check Dialog ──────────────────────────────────────────────────

interface SpaceCheckDialogProps {
  issues: SpaceIssueInfo[];
  onClose: () => void;
}

function SpaceCheckDialog({ issues, onClose }: SpaceCheckDialogProps) {
  const { t } = useI18n();
  return (
    <div className="dialog-overlay" onClick={onClose}>
      <div className="dialog" onClick={(e) => e.stopPropagation()} style={{ maxWidth: 560 }}>
        <div className="dialog-header">
          <h3 style={{ color: "#ef4444" }}>{t.jobs.spaceCheckTitle}</h3>
          <button className="dialog-close" onClick={onClose}>&times;</button>
        </div>
        <div className="dialog-body">
          <p style={{ marginBottom: 16, color: "#a1a1aa" }}>{t.jobs.spaceCheckDesc}</p>
          <table className="conflict-table" style={{ width: "100%" }}>
            <thead>
              <tr>
                <th style={{ textAlign: "left" }}>{t.jobs.spaceCheckDest}</th>
                <th style={{ textAlign: "right" }}>{t.jobs.spaceCheckAvailable}</th>
                <th style={{ textAlign: "right" }}>{t.jobs.spaceCheckRequired}</th>
                <th style={{ textAlign: "right" }}>{t.jobs.spaceCheckDeficit}</th>
              </tr>
            </thead>
            <tbody>
              {issues.map((issue, i) => (
                <tr key={i}>
                  <td style={{ fontSize: 13, wordBreak: "break-all" }}>{issue.path}</td>
                  <td style={{ textAlign: "right", color: "#f59e0b" }}>{formatBytes(issue.availableBytes)}</td>
                  <td style={{ textAlign: "right" }}>{formatBytes(issue.requiredBytes)}</td>
                  <td style={{ textAlign: "right", color: "#ef4444", fontWeight: 600 }}>-{formatBytes(issue.deficitBytes)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <div className="dialog-footer" style={{ justifyContent: "flex-end" }}>
          <button className="btn-primary" onClick={onClose}>OK</button>
        </div>
      </div>
    </div>
  );
}

// ─── Conflict Resolution Dialog ──────────────────────────────────────────

interface ConflictResolutionDialogProps {
  conflicts: FileConflict[];
  onResolve: (resolutions: ConflictResolution[]) => void;
  onCancel: () => void;
}

function ConflictResolutionDialog({ conflicts, onResolve, onCancel }: ConflictResolutionDialogProps) {
  const { t } = useI18n();
  const [actions, setActions] = useState<Map<string, ConflictAction>>(
    () => new Map(conflicts.map((c) => [`${c.relPath}::${c.destPath}`, "skip"]))
  );

  const setAction = (key: string, action: ConflictAction) => {
    setActions((prev) => {
      const next = new Map(prev);
      next.set(key, action);
      return next;
    });
  };

  const applyAll = (action: ConflictAction) => {
    setActions(new Map(conflicts.map((c) => [`${c.relPath}::${c.destPath}`, action])));
  };

  const handleProceed = () => {
    const resolutions: ConflictResolution[] = conflicts.map((c) => ({
      relPath: c.relPath,
      destPath: c.destPath,
      action: actions.get(`${c.relPath}::${c.destPath}`) || "skip",
    }));
    onResolve(resolutions);
  };

  return (
    <div className="dialog-overlay" onClick={onCancel}>
      <div className="dialog dialog--wide" onClick={(e) => e.stopPropagation()}>
        <div className="dialog-header">
          <h3>{t.jobs.conflictsDetected}</h3>
          <button className="dialog-close" onClick={onCancel}>&times;</button>
        </div>

        <div className="dialog-body">
          <p className="conflict-desc">{t.jobs.conflictsDesc}</p>
          <p className="conflict-count">
            {t.jobs.conflictCount.replace("{count}", String(conflicts.length))}
          </p>

          {/* Apply-all buttons */}
          <div className="conflict-apply-all">
            <span>{t.jobs.conflictApplyAll}:</span>
            <button className="btn-sm btn-secondary" onClick={() => applyAll("skip")}>{t.jobs.conflictSkip}</button>
            <button className="btn-sm btn-secondary" onClick={() => applyAll("overwrite")}>{t.jobs.conflictOverwrite}</button>
            <button className="btn-sm btn-secondary" onClick={() => applyAll("keepBoth")}>{t.jobs.conflictKeepBoth}</button>
          </div>

          {/* Conflict table */}
          <div className="conflict-table-wrapper">
            <table className="conflict-table">
              <thead>
                <tr>
                  <th>{t.jobs.conflictFile}</th>
                  <th>{t.jobs.conflictSource}</th>
                  <th>{t.jobs.conflictDest}</th>
                  <th>{t.jobs.conflictAction}</th>
                </tr>
              </thead>
              <tbody>
                {conflicts.map((c) => {
                  const key = `${c.relPath}::${c.destPath}`;
                  const currentAction = actions.get(key) || "skip";
                  const destName = c.destPath ? c.destPath.split("/").slice(-2).join("/") : "—";
                  return (
                    <tr key={key} className={c.sameSize ? "" : "conflict-row--diff"}>
                      <td className="conflict-cell-file" title={c.relPath}>
                        <span className="conflict-filename">{c.relPath}</span>
                        <span className={`conflict-size-badge ${c.sameSize ? "conflict-size-same" : "conflict-size-diff"}`}>
                          {c.sameSize ? t.jobs.conflictSameSize : t.jobs.conflictDiffSize}
                        </span>
                        {c.sameHash !== null && c.sameSize && (
                          <span className={`conflict-size-badge ${c.sameHash ? "conflict-size-same" : "conflict-size-diff"}`}>
                            {c.sameHash ? t.jobs.conflictSameHash : t.jobs.conflictDiffHash}
                          </span>
                        )}
                      </td>
                      <td className="conflict-cell-info">
                        <span>{formatBytes(c.sourceSize)}</span>
                        {c.sourceModified && <span className="conflict-date">{c.sourceModified}</span>}
                        {c.sourceHash && <span className="conflict-date" title={c.sourceHash}>XXH64: {c.sourceHash.slice(0, 12)}…</span>}
                      </td>
                      <td className="conflict-cell-info" title={c.destPath}>
                        <span>{formatBytes(c.destSize)}</span>
                        {c.destModified && <span className="conflict-date">{c.destModified}</span>}
                        {c.destHash && <span className="conflict-date" title={c.destHash}>XXH64: {c.destHash.slice(0, 12)}…</span>}
                        <span className="conflict-dest-name">{destName}</span>
                      </td>
                      <td className="conflict-cell-action">
                        <select
                          value={currentAction}
                          onChange={(e) => setAction(key, e.target.value as ConflictAction)}
                          className="conflict-select"
                        >
                          <option value="skip">{t.jobs.conflictSkip}</option>
                          <option value="overwrite">{t.jobs.conflictOverwrite}</option>
                          <option value="keepBoth">{t.jobs.conflictKeepBoth}</option>
                        </select>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </div>

        <div className="dialog-footer">
          <button className="btn-secondary" onClick={onCancel}>{t.common.cancel}</button>
          <button className="btn-primary" onClick={handleProceed}>{t.jobs.conflictProceed}</button>
        </div>
      </div>
    </div>
  );
}

// ─── New Offload Dialog ─────────────────────────────────────────────────

interface NewOffloadDialogProps {
  onStart: (request: StartOffloadRequest) => void;
  onCancel: () => void;
  initialSourcePath?: string;
}

function NewOffloadDialog({ onStart, onCancel, initialSourcePath }: NewOffloadDialogProps) {
  const { t, locale } = useI18n();
  const [name, setName] = useState(() => {
    const now = new Date();
    const dateStr = now.toLocaleDateString(locale === "zh" ? "zh-CN" : "en-US", {
      month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit",
    });
    return `${locale === "zh" ? "拷贝" : "Offload"} ${dateStr}`;
  });
  const [sourcePath, setSourcePath] = useState(initialSourcePath || "");
  const [destPaths, setDestPaths] = useState<string[]>([]);
  const [presets, setPresets] = useState<WorkflowPreset[]>([]);
  const [selectedPresetId, setSelectedPresetId] = useState<string | null>(null);

  // Speed priority auto-sort
  const [speedPriority, setSpeedPriority] = useState(false);

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
    // Load default offload options from settings
    safeInvoke<CommandResult<AppSettings>>("get_settings").then((result) => {
      if (result.success && result.data) {
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

  // Move destination up/down for reorder
  const handleMoveDest = (index: number, direction: "up" | "down") => {
    const newIdx = direction === "up" ? index - 1 : index + 1;
    if (newIdx < 0 || newIdx >= destPaths.length) return;
    setDestPaths((prev) => {
      const updated = [...prev];
      [updated[index], updated[newIdx]] = [updated[newIdx], updated[index]];
      return updated;
    });
  };

  // Speed priority: sort destinations by device speed (SSD > RAID > HDD > Network)
  const sortBySpeed = useCallback(async (paths: string[]) => {
    if (paths.length < 2) return paths;
    const SPEED_RANK: Record<string, number> = { SSD: 0, SD: 1, RAID: 2, HDD: 3, Network: 4, Unknown: 5 };
    try {
      const result = await safeInvoke<CommandResult<VolumeInfoResponse[]>>("list_volumes");
      if (!result.success || !result.data) return paths;
      const volumes = result.data;
      const getType = (p: string): string => {
        // Match by mount point prefix
        const vol = volumes.find((v) => p.startsWith(v.mountPoint));
        return vol?.deviceType || "Unknown";
      };
      const sorted = [...paths].sort((a, b) => {
        const ra = SPEED_RANK[getType(a)] ?? 5;
        const rb = SPEED_RANK[getType(b)] ?? 5;
        return ra - rb;
      });
      return sorted;
    } catch {
      return paths;
    }
  }, []);

  // Auto-sort when speedPriority toggled on, or when paths change with speedPriority active
  useEffect(() => {
    if (speedPriority && destPaths.length >= 2) {
      sortBySpeed(destPaths).then((sorted) => {
        if (JSON.stringify(sorted) !== JSON.stringify(destPaths)) {
          setDestPaths(sorted);
        }
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [speedPriority]); // only trigger on toggle, not on every destPaths change

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
    if (preset.defaultDestPaths.length > 0) {
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
                <div key={`dest-${i}`} className="dest-item">
                  <div className="dest-reorder-btns">
                    <button
                      className="btn-reorder"
                      disabled={i === 0}
                      onClick={() => handleMoveDest(i, "up")}
                      title="Move up"
                    >&#9650;</button>
                    <button
                      className="btn-reorder"
                      disabled={i === destPaths.length - 1}
                      onClick={() => handleMoveDest(i, "down")}
                      title="Move down"
                    >&#9660;</button>
                  </div>
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
            {speedPriority && cascade && (
              <p className="speed-priority-hint">{t.jobs.speedPriorityHint}</p>
            )}
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
              {cascade && (
                <label className="checkbox-label speed-priority-label">
                  <input
                    type="checkbox"
                    checked={speedPriority}
                    onChange={(e) => setSpeedPriority(e.target.checked)}
                  />
                  {t.jobs.speedPriority}
                </label>
              )}
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

  // Stable ref for `t` — lets the event handler always read the latest
  // translations without restarting the listener when language changes.
  const tRef = useRef(t);
  useEffect(() => { tRef.current = t; }, [t]);

  const [jobs, setJobs] = useState<JobInfo[]>([]);
  const [activeOffloads, setActiveOffloads] = useState<
    Map<string, ActiveOffload>
  >(new Map());
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showNewDialog, setShowNewDialog] = useState(false);
  const [quickCopySource, setQuickCopySource] = useState<string | undefined>();
  const [expandedJobId, setExpandedJobId] = useState<string | null>(null);
  const [selectedJobs, setSelectedJobs] = useState<Set<string>>(new Set());
  const [showTerminateConfirm, setShowTerminateConfirm] = useState(false);
  const [terminateTarget, setTerminateTarget] = useState<string | string[] | null>(null);
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<string | string[] | null>(null);
  // Conflict detection state
  const [pendingRequest, setPendingRequest] = useState<StartOffloadRequest | null>(null);
  const [detectedConflicts, setDetectedConflicts] = useState<FileConflict[]>([]);
  const [showConflictDialog, setShowConflictDialog] = useState(false);
  // Space check state
  const [spaceIssues, setSpaceIssues] = useState<SpaceIssueInfo[]>([]);
  const [showSpaceDialog, setShowSpaceDialog] = useState(false);

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

  // Listen for quick-copy events from VolumeView
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<{ mountPoint: string }>).detail;
      if (detail?.mountPoint) {
        setQuickCopySource(detail.mountPoint);
        setShowNewDialog(true);
      }
    };
    window.addEventListener("dit-quick-copy", handler);
    return () => window.removeEventListener("dit-quick-copy", handler);
  }, []);

  // Listen for real-time offload events (only in Tauri)
  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | null = null;
    let cancelled = false;

    const setup = async () => {
      const { listen } = await import("@tauri-apps/api/event");
      const unlistenFn = await listen<OffloadEventEnvelope>("offload-event", (event) => {
        const { jobId, event: ev } = event.payload;

        setActiveOffloads((prev) => {
          const next = new Map(prev);
          let offload = next.get(jobId);

          // If the ActiveOffload doesn't exist yet (event arrived before
          // executeStartOffload registered it), auto-create a placeholder
          // so we never silently drop events.
          if (!offload) {
            offload = createActiveOffload(jobId, "");
            next.set(jobId, offload);
          }

          const updated = { ...offload };
          const now = Date.now();

          switch (ev.type) {
            case "phaseChanged":
              // Reset progress counters on phase transition to prevent
              // stale 100% from previous phase flashing in the progress bar
              if (ev.phase !== updated.phase) {
                updated.completedBytes = 0;
                updated.totalBytes = 0;
                updated.completedFiles = 0;
                updated.totalFiles = 0;
                updated.currentSpeed = 0;
                updated.lastBytesSnapshot = 0;
                updated.lastSnapshotTime = Date.now();
              }
              updated.phase = ev.phase;
              updated.phaseMessage = ev.message;
              // Populate name from the first PhaseChanged event (fixes race
              // condition where events arrive before start_offload returns)
              if (ev.name && !updated.name) {
                updated.name = ev.name;
              }
              break;

            case "sourceHashCompleted":
              updated.currentFile = `[HASH] ${ev.relPath}`;
              updated.completedFiles = ev.fileIndex + 1;
              updated.totalFiles = ev.totalFiles;
              break;

            case "fileCopyStarted":
              updated.currentFile = ev.relPath;
              break;

            case "fileCopyCompleted": {
              updated.currentFile = `[COPIED] ${ev.relPath}`;
              updated.completedFiles = ev.fileIndex + 1;
              updated.totalFiles = ev.totalFiles;
              break;
            }

            case "fileVerified":
              updated.currentFile = `${ev.verified ? "[OK]" : "[FAIL]"} ${ev.relPath}${ev.destPath ? ` → ${ev.destPath.split("/").pop() || ev.destPath}` : ""}`;
              if (!ev.verified && ev.mismatchDetail) {
                updated.warnings = [
                  ...updated.warnings,
                  `Verify failed: ${ev.relPath} - ${ev.mismatchDetail}`,
                ];
              }
              break;

            case "jobProgress": {
              // If currently paused, ignore stale progress events
              if (updated.isPaused) break;

              // Detect phase change — reset speed baseline
              const phaseChanged = ev.phase !== updated.phase;
              if (phaseChanged) {
                updated.lastBytesSnapshot = 0;
                updated.lastSnapshotTime = now;
                updated.currentSpeed = 0;
                updated.phaseMessage = ev.message || "";
              }

              updated.completedFiles = ev.completedFiles;
              updated.totalFiles = ev.totalFiles;
              updated.completedBytes = ev.completedBytes;
              updated.totalBytes = ev.totalBytes;
              updated.phase = ev.phase;
              updated.elapsedSecs = ev.elapsedSecs;
              if (ev.message) {
                updated.phaseMessage = ev.message;
              }

              // Calculate instantaneous speed on every event
              if (!phaseChanged) {
                const dt = (now - updated.lastSnapshotTime) / 1000;
                if (dt >= 1.0) {
                  // Sample once per second for both speed display and history
                  const db = ev.completedBytes - updated.lastBytesSnapshot;
                  const speed = Math.max(0, db / dt);
                  updated.currentSpeed = speed;
                  updated.lastBytesSnapshot = ev.completedBytes;
                  updated.lastSnapshotTime = now;
                  if (speed > 0) {
                    const phase = updated.phase;
                    const phaseHist = [...(updated.speedHistoryByPhase[phase] || []), speed];
                    updated.speedHistoryByPhase = {
                      ...updated.speedHistoryByPhase,
                      [phase]: phaseHist.length > 120 ? phaseHist.slice(-120) : phaseHist,
                    };
                    // Append to chronological timeline (preserves phase ordering)
                    const tl = [...updated.speedTimeline, { phase, speed }];
                    updated.speedTimeline = tl.length > 240 ? tl.slice(-240) : tl;
                  }
                }
              }
              break;
            }

            case "warning":
              updated.warnings = [...updated.warnings, ev.message];
              break;

            case "sourceReleased":
              updated.sourceReleased = true;
              updated.sourceReleasedPath = ev.sourcePath;
              break;

            case "complete": {
              const evFailedFiles = ev.failedFiles ?? 0;
              updated.failedFiles = evFailedFiles;
              if (evFailedFiles > 0) {
                // Some files failed — show as Failed, not Complete
                updated.phase = "Failed";
                updated.phaseMessage = `${evFailedFiles} ${tRef.current.jobs.filesFailed}`;
              } else {
                updated.phase = "Complete";
                updated.phaseMessage = tRef.current.jobs.phaseOffloadComplete;
              }
              updated.completedFiles = Math.max(0, ev.totalFiles - evFailedFiles);
              updated.totalFiles = ev.totalFiles;
              updated.completedBytes = ev.totalBytes;
              updated.totalBytes = ev.totalBytes;
              updated.durationSecs = ev.durationSecs;
              updated.mhlPaths = ev.mhlPaths ?? [];
              updated.currentSpeed = 0;
              loadJobs();
              break;
            }

            case "error":
              updated.phase = "Failed";
              updated.error = ev.message;
              updated.phaseMessage = ev.message;
              updated.currentSpeed = 0;
              loadJobs();
              break;

            case "fileSkipped":
              updated.warnings = [...updated.warnings, `${tRef.current.jobs.skippedFiles}: ${ev.relPath} (${ev.reason})`];
              break;

            case "duplicateConflict":
              updated.warnings = [
                ...updated.warnings,
                `${tRef.current.jobs.conflicts}: ${ev.relPath} — src:${ev.sourceHash.slice(0, 8)}… vs dest:${ev.destHash.slice(0, 8)}…`,
              ];
              break;

            case "paused":
              updated.isPaused = true;
              updated.phaseMessage = tRef.current.jobs.phasePaused;
              updated.currentSpeed = 0;
              break;

            case "resumed":
              updated.isPaused = false;
              updated.phaseMessage = tRef.current.jobs.phaseResumed;
              // Reset speed baseline to avoid spike after pause gap
              updated.lastBytesSnapshot = updated.completedBytes;
              updated.lastSnapshotTime = now;
              updated.currentSpeed = 0;
              break;

            case "terminated":
              updated.phase = "Terminated";
              updated.phaseMessage = tRef.current.jobs.phaseTerminated;
              updated.currentSpeed = 0;
              updated.isPaused = false;
              loadJobs();
              break;
          }

          next.set(jobId, updated);
          return next;
        });
      });
      if (cancelled) {
        unlistenFn(); // Cleanup immediately if component already unmounted
      } else {
        unlisten = unlistenFn;
      }
    };

    setup();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [loadJobs]);

  // Actually start the offload (called directly or after conflict resolution)
  const executeStartOffload = async (request: StartOffloadRequest) => {
    try {
      setLoading(true);
      const result = await safeInvoke<CommandResult<string>>("start_offload", {
        request,
      });

      if (result.success && result.data) {
        const jobId = result.data;

        // Register ActiveOffload IMMEDIATELY so that any events already
        // queued by the backend will find their target in the map.
        // (The backend starts emitting events as soon as start_offload returns.)
        setActiveOffloads((prev) => {
          const next = new Map(prev);
          const existing = next.get(jobId);
          if (existing) {
            // Event listener created this entry early with empty name —
            // merge the real name while keeping the latest state.
            next.set(jobId, { ...existing, name: request.name });
            return next;
          }
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

  // Called when user clicks "Start" in NewOffloadDialog — detect conflicts first
  const handleStartOffload = async (request: StartOffloadRequest) => {
    setShowNewDialog(false);
    setError(null);

    try {
      setLoading(true);

      // Step 1: Scan source to get total size
      const sizeResult = await safeInvoke<CommandResult<SourceSizeInfo>>(
        "scan_source_size",
        { sourcePath: request.sourcePath }
      );
      if (!sizeResult.success || !sizeResult.data) {
        setError(sizeResult.error || "Failed to scan source");
        setLoading(false);
        return;
      }
      const { totalBytes } = sizeResult.data;

      // Step 2: Pre-flight space check on all destinations
      const destinations = request.destPaths.map((p) => [p, totalBytes] as [string, number]);
      const spaceResult = await safeInvoke<CommandResult<SpaceIssueInfo[]>>(
        "preflight_check",
        { destinations }
      );
      if (!spaceResult.success) {
        // Command itself failed — treat as blocking error
        setError(spaceResult.error || "Failed to check destination space");
        setLoading(false);
        return;
      }
      if (spaceResult.data && spaceResult.data.length > 0) {
        // Space insufficient — show blocking dialog
        setSpaceIssues(spaceResult.data);
        setShowSpaceDialog(true);
        setLoading(false);
        return;
      }

      // Step 3: Detect conflicts before starting
      const conflictResult = await safeInvoke<CommandResult<FileConflict[]>>(
        "detect_conflicts",
        { sourcePath: request.sourcePath, destPaths: request.destPaths }
      );

      if (conflictResult.success && conflictResult.data && conflictResult.data.length > 0) {
        // Conflicts found — show resolution dialog
        setPendingRequest(request);
        setDetectedConflicts(conflictResult.data);
        setShowConflictDialog(true);
        setLoading(false);
        return;
      }

      // No issues — proceed directly
      await executeStartOffload(request);
    } catch (err) {
      setError(String(err));
      setLoading(false);
    }
  };

  // Called when user resolves conflicts in the dialog
  const handleConflictResolved = async (resolutions: ConflictResolution[]) => {
    setShowConflictDialog(false);
    setDetectedConflicts([]);
    if (!pendingRequest) return;

    const requestWithResolutions: StartOffloadRequest = {
      ...pendingRequest,
      conflictResolutions: resolutions,
    };
    setPendingRequest(null);
    await executeStartOffload(requestWithResolutions);
  };

  const handleConflictCancel = () => {
    setShowConflictDialog(false);
    setDetectedConflicts([]);
    setPendingRequest(null);
    setLoading(false);
  };

  const handleRecover = async (jobId: string) => {
    try {
      setError(null);
      // Guard: prevent double-click / concurrent resume of same job
      if (activeOffloads.has(jobId)) return;
      // 1. Register ActiveOffload FIRST so the event listener can match events
      //    from the moment the backend starts emitting (fixes race condition where
      //    events were dropped because ActiveOffload wasn't in the map yet)
      const job = jobs.find((j) => j.id === jobId);
      setActiveOffloads((prev) => {
        const next = new Map(prev);
        next.set(jobId, createActiveOffload(jobId, job?.name || "Resumed Offload"));
        return next;
      });
      // 2. Then call backend resume
      const result = await safeInvoke<CommandResult<string>>("resume_offload", {
        jobId,
      });
      if (!result.success) {
        // Failed — remove the ActiveOffload we pre-registered
        setActiveOffloads((prev) => {
          const next = new Map(prev);
          next.delete(jobId);
          return next;
        });
        setError(result.error || "Recovery failed");
      }
      await loadJobs();
    } catch (err) {
      // On error, clean up the pre-registered ActiveOffload
      setActiveOffloads((prev) => {
        const next = new Map(prev);
        next.delete(jobId);
        return next;
      });
      setError(String(err));
    }
  };

  // Re-run a completed/failed/terminated job with its saved config
  const handleRerun = async (jobId: string) => {
    try {
      setError(null);
      const result = await safeInvoke<CommandResult<string>>("rerun_offload", {
        originalJobId: jobId,
      });
      if (result.success && result.data) {
        const newJobId = result.data;
        // Pre-register the new ActiveOffload
        const origJob = jobs.find((j) => j.id === jobId);
        const rerunName = origJob ? `${origJob.name} (Re-run)` : "Re-run";
        setActiveOffloads((prev) => {
          const next = new Map(prev);
          next.set(newJobId, createActiveOffload(newJobId, rerunName));
          return next;
        });
        await loadJobs();
      } else {
        setError(result.error || t.jobs.rerunFailed);
      }
    } catch (err) {
      setError(String(err));
    }
  };

  // Toast state for brief feedback messages
  const [toast, setToast] = useState<string | null>(null);
  const showToast = (msg: string) => {
    setToast(msg);
    setTimeout(() => setToast(null), 2500);
  };

  // Save a job's config as a new workflow preset
  const handleSaveAsPreset = async (jobId: string, jobName: string) => {
    const presetName = window.prompt(t.jobs.saveAsPresetName, jobName);
    if (!presetName) return;
    try {
      const result = await safeInvoke<CommandResult<unknown>>("save_job_as_preset", {
        jobId,
        presetName,
      });
      if (result.success) {
        setError(null);
        showToast(`${t.jobs.saveAsPresetSuccess}: ${presetName}`);
      } else {
        setError(result.error || t.jobs.saveAsPresetFailed);
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
      case "error":
      case "completed_with_errors":
        return "#ef4444";
      case "pending":
      case "paused":
        return "#f59e0b";
      case "terminated":
        return "#dc2626";
      case "skipped":
        return "#a3a3a3";
      case "conflict":
        return "#f97316";
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
      case "error":
      case "completed_with_errors":
        return "status-badge status-badge--failed";
      case "pending":
        return "status-badge status-badge--pending";
      case "paused":
        return "status-badge status-badge--paused";
      case "terminated":
        return "status-badge status-badge--terminated";
      case "skipped":
        return "status-badge status-badge--completed";
      case "conflict":
        return "status-badge status-badge--pending";
      default:
        return "status-badge";
    }
  };

  // Combine active offloads (sorted newest first) + DB jobs
  const activeList = useMemo(
    () => Array.from(activeOffloads.values()).sort((a, b) => b.startedAt - a.startedAt),
    [activeOffloads]
  );

  // Filter out DB jobs that already have an active offload tracker
  const dbOnlyJobs = useMemo(() => {
    const activeJobIds = new Set(activeList.map((o) => o.jobId));
    return jobs.filter((j) => !activeJobIds.has(j.id));
  }, [activeList, jobs]);

  // ─── Selection & Action Handlers ─────────────────────────────────────

  const allJobIds = [
    ...activeList.map((o) => o.jobId),
    ...dbOnlyJobs.map((j) => j.id),
  ];

  const toggleSelectJob = (jobId: string) => {
    setSelectedJobs((prev) => {
      const next = new Set(prev);
      if (next.has(jobId)) next.delete(jobId);
      else next.add(jobId);
      return next;
    });
  };

  const selectAllJobs = () => setSelectedJobs(new Set(allJobIds));
  const deselectAllJobs = () => setSelectedJobs(new Set());

  const handlePause = async (jobId: string) => {
    try {
      // Immediately update local state for instant UI feedback
      setActiveOffloads((prev) => {
        const next = new Map(prev);
        const offload = next.get(jobId);
        if (offload) {
          next.set(jobId, { ...offload, isPaused: true, phaseMessage: t.jobs.phasePaused, currentSpeed: 0 });
        }
        return next;
      });
      await safeInvoke("pause_offload", { jobId });
      await loadJobs();
    } catch (err) {
      setError(String(err));
    }
  };

  const handleResumePaused = async (jobId: string) => {
    try {
      // Immediately update local state for instant UI feedback
      setActiveOffloads((prev) => {
        const next = new Map(prev);
        const offload = next.get(jobId);
        if (offload) {
          next.set(jobId, { ...offload, isPaused: false, phaseMessage: t.jobs.phaseResuming });
        }
        return next;
      });
      const result = await safeInvoke<CommandResult<boolean>>("resume_paused_offload", { jobId });
      if (!result.success) {
        // Workflow handle gone (e.g., app restarted) — fall back to resume_offload
        // which creates a new workflow from pending DB tasks
        setActiveOffloads((prev) => {
          const next = new Map(prev);
          const job = jobs.find((j) => j.id === jobId);
          next.set(jobId, createActiveOffload(jobId, job?.name || "Resumed Offload"));
          return next;
        });
        const resumeResult = await safeInvoke<CommandResult<string>>("resume_offload", { jobId });
        if (!resumeResult.success) {
          // Both resume methods failed — clean up and show error
          setActiveOffloads((prev) => {
            const next = new Map(prev);
            next.delete(jobId);
            return next;
          });
          setError(resumeResult.error || "Resume failed");
        }
      }
      await loadJobs();
    } catch (err) {
      setError(String(err));
    }
  };

  const handleTerminateConfirm = (target: string | string[]) => {
    setTerminateTarget(target);
    setShowTerminateConfirm(true);
  };

  const handleTerminateExecute = async () => {
    if (!terminateTarget) return;
    try {
      const targets = Array.isArray(terminateTarget) ? terminateTarget : [terminateTarget];
      if (Array.isArray(terminateTarget)) {
        await safeInvoke("batch_terminate", { jobIds: terminateTarget });
      } else {
        await safeInvoke("terminate_offload", { jobId: terminateTarget });
      }
      // Immediately update frontend state for responsiveness
      // (backend also emits Terminated event, but this ensures instant UI update)
      setActiveOffloads((prev) => {
        const next = new Map(prev);
        for (const jid of targets) {
          const o = next.get(jid);
          if (o) {
            next.set(jid, {
              ...o,
              phase: "Terminated" as OffloadPhase,
              phaseMessage: t.jobs.phaseTerminated,
              currentSpeed: 0,
              isPaused: false,
            });
          }
        }
        return next;
      });
      setSelectedJobs(new Set());
      await loadJobs();
    } catch (err) {
      setError(String(err));
    }
    setShowTerminateConfirm(false);
    setTerminateTarget(null);
  };

  const handleDeleteConfirm = (target: string | string[]) => {
    setDeleteTarget(target);
    setShowDeleteConfirm(true);
  };

  const handleDeleteExecute = async () => {
    if (!deleteTarget) return;
    try {
      if (Array.isArray(deleteTarget)) {
        await safeInvoke("batch_delete", { jobIds: deleteTarget });
      } else {
        await safeInvoke("delete_job", { jobId: deleteTarget });
      }
      setSelectedJobs(new Set());
      await loadJobs();
    } catch (err) {
      setError(String(err));
    }
    setShowDeleteConfirm(false);
    setDeleteTarget(null);
  };

  const handleBatchPause = async () => {
    const ids = Array.from(selectedJobs);
    if (ids.length === 0) return;
    try {
      await safeInvoke("batch_pause", { jobIds: ids });
      await loadJobs();
    } catch (err) {
      setError(String(err));
    }
  };

  const handlePauseAll = async () => {
    // Only pause active offloads (with real workflow handles).
    // DB-only jobs with "copying"/"verifying" status are orphaned (no workflow) — skip them.
    const runningIds = activeList
      .filter((o) => o.phase !== "Complete" && o.phase !== "Failed" && o.phase !== "Terminated")
      .map((o) => o.jobId);
    if (runningIds.length === 0) return;
    try {
      await safeInvoke("batch_pause", { jobIds: runningIds });
      await loadJobs();
    } catch (err) {
      setError(String(err));
    }
  };

  const handleResumeAll = async () => {
    // Resume active offloads that are paused
    const activePausedIds = activeList
      .filter((o) => o.isPaused)
      .map((o) => o.jobId);
    for (const id of activePausedIds) {
      await safeInvoke("resume_paused_offload", { jobId: id }).catch(() => {});
    }
    // Resume DB-only paused jobs
    const dbPausedIds = dbOnlyJobs
      .filter((j) => j.status === "paused")
      .map((j) => j.id);
    for (const id of dbPausedIds) {
      await safeInvoke("resume_paused_offload", { jobId: id }).catch(() => {});
    }
    await loadJobs();
  };

  const isJobActive = (status: string) =>
    ["copying", "verifying"].includes(status);

  const isJobPaused = (status: string) => status === "paused";

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
          onCancel={() => { setShowNewDialog(false); setQuickCopySource(undefined); }}
          initialSourcePath={quickCopySource}
        />
      )}

      {/* Space Check Dialog */}
      {showSpaceDialog && spaceIssues.length > 0 && (
        <SpaceCheckDialog
          issues={spaceIssues}
          onClose={() => { setShowSpaceDialog(false); setSpaceIssues([]); }}
        />
      )}

      {/* Conflict Resolution Dialog */}
      {showConflictDialog && detectedConflicts.length > 0 && (
        <ConflictResolutionDialog
          conflicts={detectedConflicts}
          onResolve={handleConflictResolved}
          onCancel={handleConflictCancel}
        />
      )}

      {/* Job Toolbar */}
      {(activeList.length > 0 || dbOnlyJobs.length > 0) && (
        <div className="job-toolbar">
          <div className="job-toolbar-group">
            <button className="btn-toolbar" onClick={selectAllJobs}>
              {t.jobs.selectAll}
            </button>
            <button className="btn-toolbar" onClick={deselectAllJobs}>
              {t.jobs.deselectAll}
            </button>
            {selectedJobs.size > 0 && (
              <>
                <div className="job-toolbar-divider" />
                <button className="btn-toolbar btn-toolbar--warning" onClick={handleBatchPause}>
                  {t.jobs.pauseSelected} ({selectedJobs.size})
                </button>
                <button
                  className="btn-toolbar btn-toolbar--danger"
                  onClick={() => handleTerminateConfirm(Array.from(selectedJobs))}
                >
                  {t.jobs.terminateSelected} ({selectedJobs.size})
                </button>
                <button
                  className="btn-toolbar btn-toolbar--danger"
                  onClick={() => handleDeleteConfirm(Array.from(selectedJobs))}
                >
                  {t.jobs.deleteSelected} ({selectedJobs.size})
                </button>
              </>
            )}
          </div>
          <div className="job-toolbar-group">
            <button className="btn-toolbar" onClick={handlePauseAll}>
              {t.jobs.pauseAll}
            </button>
            <button className="btn-toolbar" onClick={handleResumeAll}>
              {t.jobs.resumeAll}
            </button>
            <button
              className="btn-toolbar btn-toolbar--danger"
              onClick={() => handleTerminateConfirm(allJobIds)}
            >
              {t.jobs.terminateAll}
            </button>
          </div>
        </div>
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
              offload.phase !== "Complete" && offload.phase !== "Failed" && offload.phase !== "Terminated";
            // ETA based on overall average speed (completedBytes / elapsedSecs)
            // to avoid jitter from instantaneous speed fluctuations
            const overallSpeed =
              offload.elapsedSecs > 0 && offload.completedBytes > 0
                ? offload.completedBytes / offload.elapsedSecs
                : 0;
            const eta =
              isRunning && overallSpeed > 0
                ? (offload.totalBytes - offload.completedBytes) / overallSpeed
                : 0;

            const isExpanded = expandedJobId === offload.jobId;

            return (
              <div
                key={offload.jobId}
                className={`job-card ${isRunning ? "job-card--active" : ""} ${offload.phase === "Failed" ? "job-card--failed" : ""} ${offload.phase === "Complete" ? "job-card--complete" : ""}`}
              >
                <div className="job-info">
                  <div className="job-header">
                    <input
                      type="checkbox"
                      className="job-checkbox"
                      checked={selectedJobs.has(offload.jobId)}
                      onChange={() => toggleSelectJob(offload.jobId)}
                    />
                    <span className="job-name">{offload.name || `Offload ${offload.jobId.slice(0, 8)}…`}</span>
                    <div className="job-header-right">
                      <span
                        className="job-phase-badge"
                        style={{
                          backgroundColor: phaseInfo.color + "22",
                          color: phaseInfo.color,
                          borderColor: phaseInfo.color + "44",
                        }}
                      >
                        {isRunning && !offload.isPaused && (
                          <span className="pulse-dot" style={{ backgroundColor: phaseInfo.color }} />
                        )}
                        {offload.isPaused ? t.jobs.statusPaused : phaseInfo.label}
                      </span>
                      {isRunning && !offload.isPaused && (
                        <>
                          <button className="btn-pause" onClick={() => handlePause(offload.jobId)} title={t.jobs.pause}>
                            &#x23F8;
                          </button>
                          <button className="btn-terminate" onClick={() => handleTerminateConfirm(offload.jobId)} title={t.jobs.terminate}>
                            &#x23F9;
                          </button>
                        </>
                      )}
                      {isRunning && offload.isPaused && (
                        <>
                          <button className="btn-resume" onClick={() => handleResumePaused(offload.jobId)} title={t.jobs.resume}>
                            &#x25B6;
                          </button>
                          <button className="btn-terminate" onClick={() => handleTerminateConfirm(offload.jobId)} title={t.jobs.terminate}>
                            &#x23F9;
                          </button>
                        </>
                      )}
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

                    {/* Unified speed chart (timeline-based, handles phase repeats like Copying→Verify→Copying) */}
                    {offload.speedTimeline.length > 0 && (() => {
                      const timeline = offload.speedTimeline;

                      // Split timeline into contiguous segments by phase transitions
                      type Segment = { phase: string; color: string; label: string; data: number[]; offset: number };
                      const segments: Segment[] = [];
                      let segStart = 0;
                      for (let i = 0; i <= timeline.length; i++) {
                        if (i === timeline.length || (i > 0 && timeline[i].phase !== timeline[i - 1].phase)) {
                          const phase = timeline[segStart].phase;
                          const info = PHASE_INFO[phase as OffloadPhase];
                          const raw = timeline.slice(segStart, i).map((p) => p.speed);
                          // Smooth
                          const WIN = 5;
                          const smoothed = raw.map((_, idx) => {
                            const s = Math.max(0, idx - Math.floor(WIN / 2));
                            const e = Math.min(raw.length, idx + Math.floor(WIN / 2) + 1);
                            let sum = 0;
                            for (let j = s; j < e; j++) sum += raw[j];
                            return sum / (e - s);
                          });
                          const data = smoothed.length === 1 ? [smoothed[0], smoothed[0]] : smoothed;
                          segments.push({
                            phase,
                            color: info?.color || "#3b82f6",
                            label: info?.label || phase,
                            data,
                            offset: segStart,
                          });
                          segStart = i;
                        }
                      }

                      // Deduplicated legend entries (same phase may appear multiple times as segments)
                      const legendPhases = new Map<string, { color: string; label: string }>();
                      for (const seg of segments) {
                        if (!legendPhases.has(seg.phase)) {
                          legendPhases.set(seg.phase, { color: seg.color, label: seg.label });
                        }
                      }

                      const totalLen = timeline.length;
                      const globalMax = Math.max(...segments.flatMap((s) => s.data), 1);
                      const W = 400, H = 70, PAD = 4;
                      const plotH = H - PAD * 2;
                      const plotW = W - PAD * 2;
                      const step = plotW / Math.max(totalLen - 1, 1);

                      return (
                        <div className="speed-chart">
                          <div className="speed-chart-legend">
                            {Array.from(legendPhases.entries()).map(([phase, { color, label }]) => (
                              <span key={phase} className="speed-chart-legend-item">
                                <span className="speed-chart-legend-dot" style={{ backgroundColor: color }} />
                                <span style={{ color: phase === offload.phase ? color : "#71717a" }}>
                                  {label}
                                </span>
                              </span>
                            ))}
                          </div>
                          <svg viewBox={`0 0 ${W} ${H}`} className="speed-chart-svg" preserveAspectRatio="none">
                            <defs>
                              {segments.map((seg, si) => (
                                <linearGradient key={si} id={`sg-${offload.jobId}-${si}`} x1="0" y1="0" x2="0" y2="1">
                                  <stop offset="0%" stopColor={seg.color} stopOpacity={seg.phase === offload.phase ? 0.3 : 0.1} />
                                  <stop offset="100%" stopColor={seg.color} stopOpacity="0.02" />
                                </linearGradient>
                              ))}
                            </defs>
                            {segments.map((seg, si) => {
                              const pts = seg.data.map((v, idx) =>
                                `${PAD + (seg.offset + idx) * step},${PAD + plotH - (v / globalMax) * plotH}`
                              ).join(" ");
                              const areaPts = `${PAD + seg.offset * step},${PAD + plotH} ${pts} ${PAD + (seg.offset + seg.data.length - 1) * step},${PAD + plotH}`;
                              const isLast = si === segments.length - 1;
                              const isActive = seg.phase === offload.phase;
                              return (
                                <g key={si}>
                                  <polyline points={areaPts} fill={`url(#sg-${offload.jobId}-${si})`} stroke="none" vectorEffect="non-scaling-stroke" />
                                  <polyline points={pts} fill="none" stroke={seg.color} strokeWidth={isActive ? "1.5" : "1"} strokeLinejoin="round" vectorEffect="non-scaling-stroke" opacity={isActive ? 1 : 0.5} />
                                  {isLast && seg.data.length > 0 && (
                                    <circle
                                      cx={PAD + (seg.offset + seg.data.length - 1) * step}
                                      cy={PAD + plotH - (seg.data[seg.data.length - 1] / globalMax) * plotH}
                                      r="2.5" fill={seg.color} vectorEffect="non-scaling-stroke"
                                    />
                                  )}
                                </g>
                              );
                            })}
                          </svg>
                          <div className="speed-chart-current">
                            <span className="speed-chart-value">
                              {formatSpeed(offload.currentSpeed)}
                            </span>
                            <span className="speed-chart-label">
                              {t.jobs.speed}
                            </span>
                          </div>
                        </div>
                      );
                    })()}

                    {offload.phaseMessage && (
                      <div className="job-detail-phase-msg">{offload.phaseMessage}</div>
                    )}

                    {/* Source released notice (cascade mode) */}
                    {offload.sourceReleased && offload.phase !== "Complete" && offload.phase !== "Failed" && (
                      <div className="source-released-notice">
                        <span className="source-released-icon">&#x23CF;</span>
                        {t.jobs.sourceReleased}
                      </div>
                    )}
                  </div>
                )}

                {/* Completion summary */}
                {offload.phase === "Complete" && (
                  <div className="job-complete-summary">
                    <span className="complete-check">&#x2713;</span>
                    {offload.completedFiles} {t.jobs.filesCopiedIn}{" "}
                    {formatDuration(offload.durationSecs || offload.elapsedSecs)}
                    {offload.totalBytes > 0 && (
                      <> &mdash; {formatBytes(offload.totalBytes)} {t.common.total}</>
                    )}
                    {(offload.mhlPaths?.length ?? 0) > 0 && (
                      <span className="mhl-badge">MHL</span>
                    )}
                  </div>
                )}

                {/* Failed summary with recover button */}
                {offload.phase === "Failed" && offload.failedFiles > 0 && (
                  <div className="job-failed-summary">
                    <span className="failed-icon">&#x2717;</span>
                    {offload.failedFiles} {t.jobs.filesFailed}
                    {offload.completedFiles > 0 && (
                      <>, {offload.completedFiles} {t.jobs.filesSucceeded}</>
                    )}
                    <button
                      className="btn-recover btn-sm"
                      onClick={() => handleRecover(offload.jobId)}
                      style={{ marginLeft: 12 }}
                    >
                      {t.common.recover}
                    </button>
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
                    <input
                      type="checkbox"
                      className="job-checkbox"
                      checked={selectedJobs.has(job.id)}
                      onChange={() => toggleSelectJob(job.id)}
                    />
                    <span className="job-name">{job.name || `Offload ${job.id.slice(0, 8)}…`}</span>
                    <div className="job-header-right">
                      <span className={getStatusBadgeClass(job.status)}>
                        {translateStatus(job.status, t)}
                      </span>
                      {/* DB-only jobs are orphaned (no active workflow).
                          Always show recover button — never "continue" (workflow is gone). */}
                      {(isJobActive(job.status) || isJobPaused(job.status)) && (
                        <button className="btn-recover" onClick={() => handleRecover(job.id)} title={t.common.recover}>&#x25B6;</button>
                      )}
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
                  {(job.status === "pending" || job.status === "failed" || isJobActive(job.status) || isJobPaused(job.status) || job.failedTasks > 0) &&
                    job.status !== "completed" &&
                    job.status !== "terminated" && (
                      <button
                        className="btn-recover btn-sm"
                        onClick={() => handleRecover(job.id)}
                      >
                        {t.common.recover}
                      </button>
                    )}
                  {(job.status === "completed" || job.status === "failed" || job.status === "terminated" || job.status === "completed_with_errors") && (
                    <>
                      <button
                        className="btn-rerun btn-sm"
                        onClick={() => handleRerun(job.id)}
                        title={t.jobs.rerun}
                      >
                        {t.jobs.rerun}
                      </button>
                      <button
                        className="btn-save-preset btn-sm"
                        onClick={() => handleSaveAsPreset(job.id, job.name)}
                        title={t.jobs.saveAsPreset}
                      >
                        {t.jobs.saveAsPreset}
                      </button>
                    </>
                  )}
                  <button
                    className="btn-delete"
                    onClick={() => handleDeleteConfirm(job.id)}
                    title={t.common.delete}
                  >
                    {t.common.delete}
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* Terminate Confirmation Dialog */}
      {showTerminateConfirm && (
        <div className="confirm-overlay" onClick={() => setShowTerminateConfirm(false)}>
          <div className="confirm-dialog" onClick={(e) => e.stopPropagation()}>
            <h3>{t.jobs.confirmTerminate}</h3>
            <p>
              {Array.isArray(terminateTarget)
                ? t.jobs.confirmTerminateBatchMsg
                : t.jobs.confirmTerminateMsg}
            </p>
            <div className="confirm-actions">
              <button className="btn-secondary" onClick={() => setShowTerminateConfirm(false)}>
                {t.common.cancel}
              </button>
              <button className="btn-confirm-terminate" onClick={handleTerminateExecute}>
                {t.jobs.terminateBtn}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Delete Confirmation Dialog */}
      {showDeleteConfirm && (
        <div className="confirm-overlay" onClick={() => setShowDeleteConfirm(false)}>
          <div className="confirm-dialog" onClick={(e) => e.stopPropagation()}>
            <h3>{t.jobs.confirmDelete}</h3>
            <p>
              {Array.isArray(deleteTarget)
                ? t.jobs.confirmDeleteBatchMsg
                : t.jobs.confirmDeleteMsg}
            </p>
            <div className="confirm-actions">
              <button className="btn-secondary" onClick={() => setShowDeleteConfirm(false)}>
                {t.common.cancel}
              </button>
              <button className="btn-confirm-delete" onClick={handleDeleteExecute}>
                {t.jobs.deleteBtn}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Toast notification */}
      {toast && (
        <div className="toast-notification">{toast}</div>
      )}
    </div>
  );
}
