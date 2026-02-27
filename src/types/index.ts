// DIT Pro — Core TypeScript types

export type CopyTaskStatus =
  | "pending"
  | "copying"
  | "verifying"
  | "completed"
  | "failed"
  | "paused"
  | "terminated"
  | "skipped"
  | "conflict";

export type DeviceType = "HDD" | "SSD" | "RAID" | "Network" | "Unknown";

export type HashAlgorithm = "XXH64" | "XXH3" | "XXH128" | "SHA256" | "MD5";

export interface CopyTask {
  id: string;
  sourcePath: string;
  destPaths: string[];
  fileSize: number;
  status: CopyTaskStatus;
  progress: number;
  hashResults?: HashResult[];
}

export interface HashResult {
  algorithm: HashAlgorithm;
  hexDigest: string;
}

export interface Job {
  id: string;
  name: string;
  status: string;
  sourcePath: string;
  tasks: CopyTask[];
  createdAt: string;
  totalBytes: number;
  copiedBytes: number;
}

export interface VolumeInfo {
  id: string;
  name: string;
  mountPoint: string;
  totalBytes: number;
  availableBytes: number;
  deviceType: DeviceType;
  serialNumber?: string;
  isMounted: boolean;
}

export type ViewType = "jobs" | "volumes" | "presets" | "reports" | "settings";

// ─── Backend Response Types ───────────────────────────────────────────────

/** Structured error info from Rust backend */
export interface ErrorInfo {
  code: string; // "E1001"
  category: string; // "COPY"
  severity: string; // "critical" | "error" | "warning" | "info"
  message: string;
  context?: Record<string, unknown> | null;
}

/** Standard command result from Rust backend */
export interface CommandResult<T> {
  success: boolean;
  data: T | null;
  error: string | null;
  errorInfo?: ErrorInfo | null;
}

/** Version info from Rust backend */
export interface VersionInfo {
  version: string;
  preRelease: string | null;
  buildMeta: string | null;
  channel: "dev" | "alpha" | "beta" | "rc" | "stable";
  fullString: string;
  gitHash: string | null;
  buildTime: string | null;
}

/** Error log entry from database */
export interface ErrorLogEntry {
  id: number;
  timestamp: string;
  errorCode: string;
  severity: string;
  category: string;
  module: string;
  message: string;
  contextJson: string | null;
  jobId: string | null;
  resolved: boolean;
  resolvedAt: string | null;
  appVersion: string | null;
}

/** Error log summary counts */
export interface ErrorLogSummary {
  total: number;
  critical: number;
  error: number;
  warning: number;
  info: number;
  unresolved: number;
}

/** Error log query filter */
export interface ErrorLogFilter {
  severity?: string;
  category?: string;
  jobId?: string;
  resolved?: boolean;
  limit?: number;
  offset?: number;
}

/** Job info from backend */
export interface JobInfo {
  id: string;
  name: string;
  status: string;
  sourcePath: string;
  totalTasks: number;
  completedTasks: number;
  failedTasks: number;
  totalBytes: number;
  copiedBytes: number;
  progressPercent: number;
}

/** Volume info from backend */
export interface VolumeInfoResponse {
  id: string;
  name: string;
  mountPoint: string;
  totalBytes: number;
  availableBytes: number;
  deviceType: string;
  serialNumber: string | null;
  isMounted: boolean;
  usagePercent: number;
  isLow: boolean;
  isCritical: boolean;
}

/** Space issue from pre-flight check */
export interface SpaceIssue {
  path: string;
  availableBytes: number;
  requiredBytes: number;
  deficitBytes: number;
}

/** MHL chain verification result */
export interface MhlChainVerifyResult {
  generation: number;
  valid: boolean;
}

// ─── Offload Workflow Types ──────────────────────────────────────────────

export type OffloadPhase =
  | "PreFlight"
  | "SourceVerify"
  | "Copying"
  | "Cascading"
  | "Verifying"
  | "Sealing"
  | "Complete"
  | "Failed"
  | "Terminated";

/** Envelope wrapping OffloadEvent with job_id for demuxing concurrent jobs */
export interface OffloadEventEnvelope {
  jobId: string;
  event: OffloadEvent;
}

/** Events emitted by the offload workflow via Tauri event system */
export type OffloadEvent =
  | { type: "phaseChanged"; phase: OffloadPhase; message: string }
  | {
      type: "sourceHashCompleted";
      relPath: string;
      hashes: HashResult[];
      fileIndex: number;
      totalFiles: number;
    }
  | {
      type: "fileCopyStarted";
      relPath: string;
      fileSize: number;
      destCount: number;
    }
  | {
      type: "fileCopyCompleted";
      relPath: string;
      fileSize: number;
      hashes: HashResult[];
      fileIndex: number;
      totalFiles: number;
    }
  | {
      type: "fileVerified";
      relPath: string;
      destPath: string;
      verified: boolean;
      mismatchDetail: string | null;
    }
  | {
      type: "jobProgress";
      completedFiles: number;
      totalFiles: number;
      completedBytes: number;
      totalBytes: number;
      phase: OffloadPhase;
      elapsedSecs: number;
      /** Optional per-file status message */
      message?: string;
    }
  | { type: "warning"; message: string }
  | {
      type: "complete";
      totalFiles: number;
      totalBytes: number;
      durationSecs: number;
      mhlPaths: string[];
    }
  | { type: "error"; message: string }
  | { type: "fileSkipped"; relPath: string; reason: string }
  | { type: "duplicateConflict"; relPath: string; sourceHash: string; destHash: string }
  | { type: "paused" }
  | { type: "resumed" }
  | { type: "terminated" };

/** Request to start an offload workflow */
export interface StartOffloadRequest {
  name: string;
  sourcePath: string;
  destPaths: string[];
  hashAlgorithms?: string[];
  sourceVerify?: boolean;
  postVerify?: boolean;
  generateMhl?: boolean;
  cascade?: boolean;
  conflictResolutions?: ConflictResolution[];
}

// ─── Conflict Detection Types ────────────────────────────────────────────

/** A file conflict detected before copy (destination file already exists) */
export interface FileConflict {
  relPath: string;
  sourceSize: number;
  sourceModified: string | null;
  destPath: string;
  destSize: number;
  destModified: string | null;
  sameSize: boolean;
  /** true if same_size AND content hashes match (XXH64) */
  sameHash: boolean | null;
  /** Source file hash (XXH64, hex) */
  sourceHash: string | null;
  /** Dest file hash (XXH64, hex) */
  destHash: string | null;
}

/** User's decision for a conflicting file */
export type ConflictAction = "skip" | "overwrite" | "keepBoth";

/** A single conflict resolution from the user */
export interface ConflictResolution {
  relPath: string;
  destPath: string;
  action: ConflictAction;
}

// ─── Settings Types ──────────────────────────────────────────────────────

export interface DeviceIoConfig {
  maxConcurrent: number;
  bufferSizeMb: number;
}

export interface IoSchedulingSettings {
  hdd: DeviceIoConfig;
  ssd: DeviceIoConfig;
  raid: DeviceIoConfig;
  network: DeviceIoConfig;
}

export interface OffloadDefaults {
  sourceVerify: boolean;
  postVerify: boolean;
  generateMhl: boolean;
  bufferSize: number;
  maxRetries: number;
  cascade: boolean;
}

export interface EmailSettings {
  enabled: boolean;
  smtpHost: string;
  smtpPort: number;
  smtpUsername: string;
  smtpPasswordSet: boolean;
  fromAddress: string;
  toAddress: string;
  useTls: boolean;
}

export interface ReportSettings {
  defaultExportPath: string;
  exportFormat: string;
  askPathEachTime: boolean;
  askFormatEachTime: boolean;
}

export interface AppSettings {
  offload: OffloadDefaults;
  ioScheduling: IoSchedulingSettings;
  email: EmailSettings;
  report: ReportSettings;
}

// ─── Workflow Preset Types ──────────────────────────────────────────────

export interface WorkflowPreset {
  id: string;
  name: string;
  description: string;
  hashAlgorithms: string[];
  sourceVerify: boolean;
  postVerify: boolean;
  generateMhl: boolean;
  bufferSize: number;
  maxRetries: number;
  cascade: boolean;
  defaultDestPaths: string[];
  createdAt: string;
  updatedAt: string;
}

// ─── Report Types ──────────────────────────────────────────────────────

export interface JobSummary {
  jobId: string;
  jobName: string;
  status: string;
  sourcePath: string;
  totalFiles: number;
  completedFiles: number;
  failedFiles: number;
  totalBytes: number;
  completedBytes: number;
  createdAt: string;
}

export interface TaskDetail {
  sourcePath: string;
  destPath: string;
  fileSize: number;
  status: string;
  hashXxh64: string | null;
  hashSha256: string | null;
}

export interface DayReport {
  date: string;
  jobs: JobSummary[];
  totalFiles: number;
  totalBytes: number;
  totalJobs: number;
  completedJobs: number;
  failedJobs: number;
}

export interface JobReport {
  summary: JobSummary;
  tasks: TaskDetail[];
  destPaths: string[];
}
