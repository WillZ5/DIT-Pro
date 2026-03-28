// Tauri compatibility layer — provides mock data in browser preview mode
import type { AppSettings, CommandResult, WorkflowPreset } from "../types";
import {
  MOCK_JOBS,
  MOCK_VOLUMES,
  MOCK_PRESETS,
  MOCK_SETTINGS,
  MOCK_DAY_REPORT,
  MOCK_JOB_REPORT,
  MOCK_RUSHES_LOG,
} from "./mockData";

/** True when running inside the Tauri desktop shell */
export function isTauri(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

/** Whether we're in browser-only demo mode */
export const IS_DEMO = !isTauri();

function ok<T>(data: T): CommandResult<T> {
  return { success: true, data, error: null };
}

function fail<T>(msg: string): CommandResult<T> {
  return { success: false, data: null, error: msg };
}

/** In-memory mutable store for demo mode */
const demoStore = {
  presets: [...MOCK_PRESETS],
  settings: { ...MOCK_SETTINGS },
};

interface MockInvokeArgs {
  settings?: AppSettings;
  presetData?: WorkflowPreset;
  presetId?: string;
  jobId?: string;
  jobIds?: string[];
}

/** Mock command handler */
function mockInvoke<T>(cmd: string, args?: MockInvokeArgs): T {
  switch (cmd) {
    case "get_app_version":
      return {
        version: "1.3.0",
        preRelease: "beta",
        buildMeta: null,
        channel: "beta",
        fullString: "1.3.0-beta",
        gitHash: null,
        buildTime: null,
      } as T;
    case "list_jobs":
      return ok(MOCK_JOBS) as T;
    case "list_volumes":
      return ok(MOCK_VOLUMES) as T;
    case "list_presets":
      return ok(demoStore.presets) as T;
    case "get_settings":
      return ok(demoStore.settings) as T;
    case "save_settings":
      if (args?.settings) demoStore.settings = args.settings;
      return ok(true) as T;
    case "get_report_dates":
      return ok([MOCK_DAY_REPORT.date]) as T;
    case "get_day_report":
      return ok(MOCK_DAY_REPORT) as T;
    case "get_job_report":
      return ok(MOCK_JOB_REPORT) as T;
    case "create_preset": {
      if (!args?.presetData) {
        return fail("Missing presetData") as T;
      }
      const preset = {
        ...args.presetData,
        id: `preset-${Date.now()}`,
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString(),
      } satisfies WorkflowPreset;
      demoStore.presets.push(preset);
      return ok(preset) as T;
    }
    case "update_preset": {
      if (!args?.presetData) {
        return fail("Missing presetData") as T;
      }
      const presetData = args.presetData;
      const idx = demoStore.presets.findIndex((p) => p.id === presetData.id);
      if (idx >= 0) {
        demoStore.presets[idx] = { ...presetData, updatedAt: new Date().toISOString() };
        return ok(demoStore.presets[idx]) as T;
      }
      return fail("Preset not found") as T;
    }
    case "delete_preset":
      demoStore.presets = demoStore.presets.filter((p) => p.id !== args?.presetId);
      return ok(true) as T;
    case "start_offload":
      return ok(`demo-job-${Date.now()}`) as T;
    case "recover_job":
      return ok(MOCK_JOBS[0]) as T;
    case "resume_offload":
      return ok(args?.jobId || `demo-resume-${Date.now()}`) as T;
    case "export_day_report":
    case "export_job_report":
      return ok("/tmp/dit-report-demo.html") as T;
    case "pause_offload":
      return ok(true) as T;
    case "resume_paused_offload":
      return ok(true) as T;
    case "terminate_offload":
      return ok(true) as T;
    case "batch_pause":
      return ok(true) as T;
    case "batch_terminate":
      return ok(true) as T;
    case "clear_logs":
      return ok(0) as T;
    case "reveal_in_finder":
      return ok(true) as T;
    case "delete_job":
      return ok(true) as T;
    case "batch_delete":
      return ok(args?.jobIds?.length ?? 0) as T;
    case "get_error_log":
      return ok([]) as T;
    case "get_error_log_summary":
      return ok({ total: 0, critical: 0, error: 0, warning: 0, info: 0, unresolved: 0 }) as T;
    case "resolve_error_entry":
      return ok(true) as T;
    case "clear_error_log_entries":
      return ok(0) as T;
    case "export_debug_bundle":
      return ok("/tmp/dit-debug-demo.json") as T;
    case "check_for_update":
      return {
        hasUpdate: false,
        latestVersion: "1.4.0-beta",
        currentVersion: "1.4.0-beta",
        releaseNotes: "Rushes Log with camera card ID, ffprobe metadata, CSV/TSV/Excel/PDF export.",
        releaseUrl: "https://github.com/WillZ5/DIT-Pro/releases/tag/v1.3.0-beta",
        downloadUrl: null,
        publishedAt: "2026-03-09T00:00:00Z",
      } as T;
    case "get_job_config":
      return ok('{"job_id":"demo","job_name":"Demo Job","source_path":"/Volumes/CARD_A","dest_paths":["/Volumes/Backup1"],"hash_algorithms":["XXH64","SHA256"],"buffer_size":1048576,"source_verify":true,"post_verify":true,"generate_mhl":true,"max_retries":3,"cascade":false,"conflict_resolutions":{}}') as T;
    case "rerun_offload":
      return ok(`demo-rerun-${Date.now()}`) as T;
    case "save_job_as_preset": {
      const preset = {
        id: `preset-${Date.now()}`,
        name: (args as { presetName?: string })?.presetName || "Saved Preset",
        description: "",
        hashAlgorithms: ["XXH64", "SHA256"],
        sourceVerify: true,
        postVerify: true,
        generateMhl: true,
        bufferSize: 4194304,
        maxRetries: 3,
        cascade: false,
        defaultDestPaths: [],
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString(),
      };
      demoStore.presets.push(preset);
      return ok(preset) as T;
    }
    case "scan_source_size":
      return ok({ totalFiles: 342, totalBytes: 453_800_000_000 }) as T;
    case "preflight_check":
      return ok([]) as T;
    case "get_rushes_log":
      return ok(MOCK_RUSHES_LOG) as T;
    case "export_rushes_log":
      return ok("/tmp/rushes-log-demo.csv") as T;
    case "copy_rushes_log_clipboard":
      return ok("Reel\tCamera\tClips\tFirst Clip\tLast Clip\tSize\tDuration\tSpeed\tStatus\tMHL\tDestinations\nA001\tARRI ALEXA Mini\t42\tA001C001_240215_R1AB.mxf\tA001C042_240215_R1AB.mxf\t453.8 GB\t47m 27s\t171.1 MB/s\tVerified\t✓\tRAID_SHUTTLE_01, RAID_SHUTTLE_02") as T;
    default:
      return ok(null) as T;
  }
}

/**
 * Drop-in replacement for Tauri's `invoke()`.
 * In browser mode, returns mock data immediately.
 */
export async function safeInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri()) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<T>(cmd, args);
  }
  // Simulate slight network delay for realism
  await new Promise((r) => setTimeout(r, 80));
  return mockInvoke<T>(cmd, args);
}

/**
 * Convert a local file path to a URL that can be loaded in the browser.
 * Uses Tauri's convertFileSrc() when running in desktop shell.
 */
export function convertPathToSrc(path: string): string {
  if (isTauri()) {
    // Note: convertFileSrc is synchronous, but we can't top-level import it in web mode.
    // As a workaround, we assume the window.__TAURI__ or tauri asset protocol is available
    // For Tauri v2, the protocol is usually asset://localhost/
    return `asset://localhost/${encodeURIComponent(path)}`;
  }
  // Mock image for demo mode
  return "https://via.placeholder.com/480x270?text=Thumbnail";
}
