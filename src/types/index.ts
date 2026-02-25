// DIT System — Core TypeScript types

export type CopyTaskStatus =
  | "pending"
  | "copying"
  | "verifying"
  | "completed"
  | "failed"
  | "skipped";

export type DeviceType = "HDD" | "SSD" | "NVMe" | "RAID" | "Network" | "Unknown";

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

export type ViewType = "jobs" | "volumes" | "reports" | "settings";

// ─── Backend Response Types ───────────────────────────────────────────────

/** Standard command result from Rust backend */
export interface CommandResult<T> {
  success: boolean;
  data: T | null;
  error: string | null;
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
  | "Verifying"
  | "Sealing"
  | "Complete"
  | "Failed";

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
    }
  | { type: "warning"; message: string }
  | {
      type: "complete";
      totalFiles: number;
      totalBytes: number;
      durationSecs: number;
      mhlPaths: string[];
    }
  | { type: "error"; message: string };

/** Request to start an offload workflow */
export interface StartOffloadRequest {
  name: string;
  sourcePath: string;
  destPaths: string[];
  hashAlgorithms?: string[];
  sourceVerify?: boolean;
  postVerify?: boolean;
  generateMhl?: boolean;
}
