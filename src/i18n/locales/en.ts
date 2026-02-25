// English translations for DIT System
const en = {
  // ─── App Shell ──────────────────────────────────────────────────────
  app: {
    title: "DIT System",
    subtitle: "Material Offload",
    demo: "DEMO",
    demoBanner:
      "Browser preview mode — using mock data. Run in Tauri for full functionality.",
    quitConfirmTitle: "Quit DIT System?",
    quitConfirmMessage: "Are you sure you want to quit DIT System?",
    quitConfirmMessageActive:
      "There are active offload jobs running. Quitting now may result in incomplete copies. Are you sure you want to quit?",
    quitConfirm: "Quit",
    quitCancel: "Cancel",
  },

  // ─── Navigation ─────────────────────────────────────────────────────
  nav: {
    jobs: "Jobs",
    volumes: "Volumes",
    presets: "Presets",
    reports: "Reports",
    settings: "Settings",
  },

  // ─── Common ─────────────────────────────────────────────────────────
  common: {
    cancel: "Cancel",
    dismiss: "Dismiss",
    close: "Close",
    save: "Save",
    delete: "Delete",
    duplicate: "Duplicate",
    browse: "Browse",
    loading: "Loading...",
    saving: "Saving...",
    saved: "Saved",
    export: "Export",
    recover: "Recover",
    files: "files",
    free: "free",
    used: "used",
    total: "Total",
  },

  // ─── Jobs View ──────────────────────────────────────────────────────
  jobs: {
    title: "Jobs",
    newOffload: "+ New Offload",
    starting: "Starting...",
    // New offload dialog
    dialogTitle: "New Offload",
    jobName: "Job Name",
    jobNamePlaceholder: "e.g. Day 1 A-Cam",
    sourceCard: "Source Card / Directory",
    sourcePlaceholder: "Select source directory...",
    destinations: "Destinations",
    selected: "selected",
    addDest: "+ Add Destination",
    primary: "Primary",
    dest: "Dest",
    workflowPreset: "Workflow Preset",
    customConfig: "Custom Configuration",
    options: "Options",
    sourceVerify: "Source Verify",
    postVerify: "Post Verify",
    generateMhl: "Generate MHL",
    cascadeCopy: "Cascade Copy",
    hashAlgorithms: "Hash Algorithms",
    startOffload: "Start Offload",
    cascadeInfo:
      'Cascade mode: Files copy to <strong>{dest}</strong> first (fastest), then cascade to {count} secondary destination(s). Source card is freed sooner.',
    // Empty state
    noJobs: "No active jobs",
    noJobsHint: 'Insert a card and click "New Offload" to start copying.',
    // Phases
    phasePreFlight: "Pre-Flight",
    phaseSourceVerify: "Hashing Source",
    phaseCopying: "Copying",
    phaseCascading: "Cascading",
    phaseVerifying: "Verifying",
    phaseSealing: "MHL Sealing",
    phaseComplete: "Complete",
    phaseFailed: "Failed",
    // Job card
    filesCopiedIn: "files copied in",
    failed: "failed",
    // Status labels (backend values → display text)
    statusCompleted: "COMPLETED",
    statusCompletedWithErrors: "COMPLETED WITH ERRORS",
    statusCopying: "COPYING",
    statusVerifying: "VERIFYING",
    statusFailed: "FAILED",
    statusPending: "PENDING",
    statusError: "ERROR",
    // Job detail dropdown
    details: "Details",
    currentFile: "Current File",
    speed: "Speed",
    elapsed: "Elapsed",
    eta: "ETA",
  },

  // ─── Volumes View ───────────────────────────────────────────────────
  volumes: {
    title: "Volumes",
    refresh: "Refresh",
    scanning: "Scanning...",
    noVolumes: "No volumes detected",
    noVolumesHint: "Connect an external drive to get started.",
    unmounted: "Unmounted",
    critical: "CRITICAL",
    low: "LOW",
    openInFinder: "Click to open in Finder",
  },

  // ─── Presets View ───────────────────────────────────────────────────
  presets: {
    title: "Workflow Presets",
    newPreset: "+ New Preset",
    noPresets: 'No presets yet. Click "+ New Preset" to create one.',
    editTitle: "Edit Preset",
    newTitle: "New Preset",
    name: "Name",
    namePlaceholder: "e.g., ARRI Daily Offload",
    description: "Description",
    descPlaceholder: "Optional description...",
    hashAlgorithms: "Hash Algorithms",
    sourceVerification: "Source Verification",
    postCopyVerification: "Post-Copy Verification",
    generateAscMhl: "Generate ASC MHL",
    cascadingCopy: "Cascading Copy",
    bufferSize: "Buffer Size",
    maxRetries: "Max Retries",
    createPreset: "Create Preset",
    saveChanges: "Save Changes",
    presetNameRequired: "Preset name is required",
    cascade: "Cascade",
    srcVerify: "SrcVerify",
    postVerifyFlag: "PostVerify",
  },

  // ─── Reports View ──────────────────────────────────────────────────
  reports: {
    title: "Reports",
    exportHtml: "Export HTML",
    exporting: "Exporting...",
    noReports: "No reports yet",
    noReportsHint: "Reports will be generated after completing offload jobs.",
    daySummary: "Day Summary",
    totalJobs: "Total Jobs",
    totalFiles: "Total Files",
    totalData: "Total Data",
    completed: "Completed",
    jobsTableTitle: "Jobs",
    colName: "Name",
    colStatus: "Status",
    colFiles: "Files",
    colSize: "Size",
    colActions: "Actions",
    detail: "Detail",
    jobDetail: "Job Detail",
    colFile: "File",
    colDestination: "Destination",
    reportSavedTo: "Report saved to:",
  },

  // ─── Settings View ─────────────────────────────────────────────────
  settings: {
    title: "Settings",
    saveSettings: "Save Settings",
    loadingSettings: "Loading settings...",
    // Hash algorithms section
    hashAlgorithmsTitle: "Hash Algorithms",
    hashAlgorithmsDesc:
      "Select which hash algorithms to use during copy verification.",
    algoXxh64Desc: "Ultra-fast, recommended default",
    algoXxh3Desc: "Next-gen, fastest available",
    algoXxh128Desc: "128-bit XXH variant",
    algoSha256Desc: "Cryptographic, high security",
    algoMd5Desc: "Legacy compatibility",
    // Offload defaults section
    offloadDefaultsTitle: "Offload Defaults",
    offloadDefaultsDesc: "Default options applied to every new offload job.",
    sourceVerification: "Source Verification",
    sourceVerifyDesc:
      "Hash source files before copying to detect read errors",
    postCopyVerification: "Post-Copy Verification",
    postVerifyDesc: "Re-read destination files and verify hashes match",
    generateAscMhl: "Generate ASC MHL",
    generateMhlDesc:
      "Create chain-of-custody manifest after successful copy",
    cascadingCopy: "Cascading Copy",
    cascadeDesc:
      "Copy to fastest destination first, then cascade to slower targets",
    bufferSize: "Buffer Size",
    bufferSizeDesc: "IO buffer size for file operations",
    bufferDefault: "(default)",
    maxRetries: "Max Retries",
    maxRetriesDesc: "Retry attempts for failed file copies",
    noRetry: "no retry",
    // IO scheduling section
    ioSchedulingTitle: "IO Scheduling",
    ioSchedulingDesc: "Per-device concurrency and buffer settings.",
    colDevice: "Device",
    colMaxConcurrent: "Max Concurrent",
    colBufferMb: "Buffer (MB)",
    deviceHdd: "Mechanical hard drive",
    deviceSsd: "Solid state drive",
    deviceNvme: "High-speed NVMe",
    deviceRaid: "RAID array",
    deviceNetwork: "Network share",
    // Email section
    emailTitle: "Email Notifications",
    emailDesc: "Send email alerts when offload jobs complete or fail.",
    enableEmail: "Enable Email Notifications",
    enableEmailDesc: "Send alerts via SMTP when jobs finish",
    smtpHost: "SMTP Host",
    port: "Port",
    tls: "TLS",
    username: "Username",
    fromAddress: "From Address",
    toAddress: "To Address",
    // Language section
    languageTitle: "Language",
    languageDesc: "Select the interface language.",
    languageEn: "English",
    languageZh: "Chinese (Simplified)",
  },
};

// Derive structure type with string values (not literal types)
type DeepStringify<T> = {
  [K in keyof T]: T[K] extends string ? string : DeepStringify<T[K]>;
};

export type TranslationKeys = DeepStringify<typeof en>;
export default en as TranslationKeys;
