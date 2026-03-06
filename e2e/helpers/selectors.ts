/**
 * CSS selectors used across E2E tests.
 * Centralised here so a single UI refactor only needs one update.
 */

// ── Layout ──────────────────────────────────────────────────────────
export const APP_ROOT = ".app";
export const SIDEBAR = "nav.sidebar";
export const MAIN_CONTENT = "main.main-content";

// ── Sidebar ─────────────────────────────────────────────────────────
export const APP_TITLE = ".app-title";
export const NAV_ITEM = "button.nav-item";
export const NAV_ITEM_ACTIVE = "button.nav-item.active";
export const NAV_LABEL = ".nav-label";
export const DEMO_BADGE = ".demo-badge";
export const DEMO_BANNER = ".demo-banner";
export const VERSION = ".version";
export const CHANNEL_BADGE = ".channel-badge";

// ── Navigation helpers (use with page.locator) ─────────────────────
export const navItem = (label: string) =>
  `button.nav-item:has(.nav-label:text("${label}"))`;
export const navItemActive = (label: string) =>
  `button.nav-item.active:has(.nav-label:text("${label}"))`;

// ── Views ───────────────────────────────────────────────────────────
export const JOBS_VIEW = ".jobs-view";
export const VOLUME_VIEW = ".volume-view";
export const PRESETS_VIEW = ".presets-layout";
export const SETTINGS_VIEW = ".settings-view";

// ── Volume cards ────────────────────────────────────────────────────
export const VOLUME_CARD = ".volume-card";
export const VOLUME_NAME = ".volume-name";
export const VOLUME_TYPE = ".volume-type";
export const VOLUME_BAR = ".volume-bar";
export const WARNING_BADGE = ".warning-badge";

// ── Preset cards ────────────────────────────────────────────────────
export const PRESET_CARD = ".preset-card";
export const PRESET_CARD_NAME = ".preset-card-name";
export const PRESET_EDITOR = ".preset-editor";
export const ALGO_CHIP = ".algo-chip";
export const ALGO_CHIP_ACTIVE = ".algo-chip.algo-chip--active";

// ── Settings ────────────────────────────────────────────────────────
export const SETTINGS_SECTION = "section.settings-section";
export const SAVE_BUTTON = "button.btn-primary";
export const IO_TABLE_ROW = ".io-table-row";

// ── Reports ─────────────────────────────────────────────────────────
export const REPORT_STAT = ".report-stat";
export const REPORT_STAT_VALUE = ".report-stat-value";
export const REPORT_TABLE = ".report-table";

// ── Dialogs ─────────────────────────────────────────────────────────
export const DIALOG_OVERLAY = ".dialog-overlay";
export const DIALOG = ".dialog";
export const DIALOG_CLOSE = ".dialog-close";
export const ABOUT_TABLE = ".about-table";

// ── Buttons ─────────────────────────────────────────────────────────
export const BTN_PRIMARY = "button.btn-primary";
export const BTN_SECONDARY = "button.btn-secondary";
export const BTN_SMALL = "button.btn-small";
