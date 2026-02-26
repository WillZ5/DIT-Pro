// Tauri compatibility layer — provides mock data in browser preview mode
import type { CommandResult } from "../types";
import {
  MOCK_JOBS,
  MOCK_VOLUMES,
  MOCK_PRESETS,
  MOCK_SETTINGS,
  MOCK_DAY_REPORT,
  MOCK_JOB_REPORT,
} from "./mockData";

/** True when running inside the Tauri desktop shell */
export function isTauri(): boolean {
  return !!(window as any).__TAURI_INTERNALS__;
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

/** Mock command handler */
function mockInvoke<T>(cmd: string, args?: Record<string, any>): T {
  switch (cmd) {
    case "get_app_version":
      return {
        version: "1.0.0",
        preRelease: "demo",
        buildMeta: null,
        channel: "dev",
        fullString: "1.0.0-demo",
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
      const preset = {
        ...args?.presetData,
        id: `preset-${Date.now()}`,
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString(),
      };
      demoStore.presets.push(preset);
      return ok(preset) as T;
    }
    case "update_preset": {
      const idx = demoStore.presets.findIndex((p) => p.id === args?.presetData?.id);
      if (idx >= 0) {
        demoStore.presets[idx] = { ...args!.presetData, updatedAt: new Date().toISOString() };
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
    default:
      return ok(null) as T;
  }
}

/**
 * Drop-in replacement for Tauri's `invoke()`.
 * In browser mode, returns mock data immediately.
 */
export async function safeInvoke<T>(cmd: string, args?: Record<string, any>): Promise<T> {
  if (isTauri()) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<T>(cmd, args);
  }
  // Simulate slight network delay for realism
  await new Promise((r) => setTimeout(r, 80));
  return mockInvoke<T>(cmd, args);
}
