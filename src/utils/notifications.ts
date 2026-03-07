/**
 * Push notification utility for DIT Pro.
 *
 * Uses @tauri-apps/plugin-notification for macOS/Windows system notifications.
 * Gracefully degrades in browser (dev) mode.
 */

import { isTauri } from "./tauriCompat";

export type NotificationEvent = "taskComplete" | "taskFailed" | "sourceReleased";

export interface NotificationSettings {
  enabled: boolean;
  taskComplete: boolean;
  taskFailed: boolean;
  sourceReleased: boolean;
}

export const DEFAULT_NOTIFICATION_SETTINGS: NotificationSettings = {
  enabled: true,
  taskComplete: true,
  taskFailed: true,
  sourceReleased: true,
};

let permissionGranted: boolean | null = null;

/**
 * Initialize notification permissions.
 * Returns true if notifications are available and permitted.
 */
export async function initNotifications(): Promise<boolean> {
  if (!isTauri()) return false;

  try {
    const { isPermissionGranted, requestPermission } = await import(
      "@tauri-apps/plugin-notification"
    );

    let granted = await isPermissionGranted();
    if (!granted) {
      const result = await requestPermission();
      granted = result === "granted";
    }
    permissionGranted = granted;
    return granted;
  } catch {
    permissionGranted = false;
    return false;
  }
}

/**
 * Check current permission status without requesting.
 */
export async function checkNotificationPermission(): Promise<boolean> {
  if (permissionGranted !== null) return permissionGranted;
  if (!isTauri()) return false;

  try {
    const { isPermissionGranted } = await import(
      "@tauri-apps/plugin-notification"
    );
    permissionGranted = await isPermissionGranted();
    return permissionGranted;
  } catch {
    return false;
  }
}

/**
 * Send a DIT Pro notification if the event is enabled.
 */
export async function sendDitNotification(
  event: NotificationEvent,
  title: string,
  body: string,
  settings: NotificationSettings
): Promise<void> {
  if (!settings.enabled || !settings[event]) return;
  if (!isTauri()) return;

  // Ensure permission is checked
  if (permissionGranted === null) {
    await checkNotificationPermission();
  }
  if (!permissionGranted) return;

  try {
    const { sendNotification } = await import(
      "@tauri-apps/plugin-notification"
    );
    sendNotification({ title, body });
  } catch {
    /* Notification not available — skip silently */
  }
}
