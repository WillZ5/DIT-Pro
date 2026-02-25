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
