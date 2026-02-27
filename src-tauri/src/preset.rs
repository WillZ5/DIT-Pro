//! Workflow Presets — Save/load reusable offload configurations.
//!
//! Presets store commonly used offload parameters so DITs can quickly
//! switch between different shooting scenarios (e.g., "Daily ARRI Offload",
//! "LTO Archive", "Quick Review Copy").
//!
//! Stored as `presets.json` in the app data directory.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A single workflow preset
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPreset {
    /// Unique identifier
    pub id: String,
    /// Human-readable name (e.g., "ARRI Daily Offload")
    pub name: String,
    /// Optional description
    #[serde(default)]
    pub description: String,
    /// Hash algorithms to use
    #[serde(default = "default_algorithms")]
    pub hash_algorithms: Vec<String>,
    /// Source verification (hash source before copy)
    #[serde(default = "default_true")]
    pub source_verify: bool,
    /// Post-copy verification (re-read and verify destinations)
    #[serde(default = "default_true")]
    pub post_verify: bool,
    /// Generate ASC MHL manifests
    #[serde(default = "default_true")]
    pub generate_mhl: bool,
    /// Buffer size in bytes
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,
    /// Max retries for failed copies
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Enable cascading copy
    #[serde(default)]
    pub cascade: bool,
    /// Optional default destination paths
    #[serde(default)]
    pub default_dest_paths: Vec<String>,
    /// Timestamp when created
    #[serde(default)]
    pub created_at: String,
    /// Timestamp when last updated
    #[serde(default)]
    pub updated_at: String,
}

fn default_algorithms() -> Vec<String> {
    vec!["XXH64".to_string(), "SHA256".to_string()]
}

fn default_true() -> bool {
    true
}

fn default_buffer_size() -> usize {
    4 * 1024 * 1024
}

fn default_max_retries() -> u32 {
    3
}

/// Container for all presets (serialized to disk)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PresetStore {
    pub presets: Vec<WorkflowPreset>,
}

// ─── Persistence ──────────────────────────────────────────────────────────

const PRESETS_FILE: &str = "presets.json";

/// Resolve the presets file path.
pub fn presets_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(PRESETS_FILE)
}

/// Load all presets from disk.
pub fn load_presets(app_data_dir: &Path) -> Result<PresetStore> {
    let path = presets_path(app_data_dir);
    if !path.exists() {
        return Ok(PresetStore::default());
    }
    let data = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read presets from {:?}", path))?;
    let store: PresetStore =
        serde_json::from_str(&data).with_context(|| "Failed to parse presets JSON")?;
    Ok(store)
}

/// Save all presets to disk (pretty-printed JSON).
pub fn save_presets(app_data_dir: &Path, store: &PresetStore) -> Result<()> {
    let path = presets_path(app_data_dir);
    let data = serde_json::to_string_pretty(store).context("Failed to serialize presets")?;
    std::fs::write(&path, data)
        .with_context(|| format!("Failed to write presets to {:?}", path))?;
    Ok(())
}

/// Create a new preset and persist.
pub fn create_preset(app_data_dir: &Path, mut preset: WorkflowPreset) -> Result<WorkflowPreset> {
    let mut store = load_presets(app_data_dir)?;

    // Assign ID if empty
    if preset.id.is_empty() {
        preset.id = uuid::Uuid::new_v4().to_string();
    }

    let now = chrono::Utc::now().to_rfc3339();
    preset.created_at = now.clone();
    preset.updated_at = now;

    store.presets.push(preset.clone());
    save_presets(app_data_dir, &store)?;

    Ok(preset)
}

/// Update an existing preset by ID.
pub fn update_preset(app_data_dir: &Path, updated: WorkflowPreset) -> Result<WorkflowPreset> {
    let mut store = load_presets(app_data_dir)?;

    let idx = store
        .presets
        .iter()
        .position(|p| p.id == updated.id)
        .ok_or_else(|| anyhow::anyhow!("Preset not found: {}", updated.id))?;

    let mut preset = updated;
    preset.updated_at = chrono::Utc::now().to_rfc3339();
    // Preserve original created_at
    preset.created_at = store.presets[idx].created_at.clone();

    store.presets[idx] = preset.clone();
    save_presets(app_data_dir, &store)?;

    Ok(preset)
}

/// Delete a preset by ID.
pub fn delete_preset(app_data_dir: &Path, preset_id: &str) -> Result<()> {
    let mut store = load_presets(app_data_dir)?;
    let before = store.presets.len();
    store.presets.retain(|p| p.id != preset_id);

    if store.presets.len() == before {
        bail!("Preset not found: {}", preset_id);
    }

    save_presets(app_data_dir, &store)?;
    Ok(())
}

// ─── Built-in Presets ─────────────────────────────────────────────────────

/// Generate built-in presets for first-time users.
pub fn builtin_presets() -> Vec<WorkflowPreset> {
    let now = chrono::Utc::now().to_rfc3339();

    vec![
        WorkflowPreset {
            id: "builtin-paranoid".to_string(),
            name: "Paranoid (Full Verify)".to_string(),
            description: "Maximum safety: source hash + copy + post-verify + MHL seal. Use for critical footage."
                .to_string(),
            hash_algorithms: vec!["XXH64".to_string(), "SHA256".to_string()],
            source_verify: true,
            post_verify: true,
            generate_mhl: true,
            buffer_size: 4 * 1024 * 1024,
            max_retries: 3,
            cascade: false,
            default_dest_paths: Vec::new(),
            created_at: now.clone(),
            updated_at: now.clone(),
        },
        WorkflowPreset {
            id: "builtin-fast".to_string(),
            name: "Fast Copy (XXH64 Only)".to_string(),
            description: "Speed-optimized: skip source verify, use fastest hash only. Good for review copies."
                .to_string(),
            hash_algorithms: vec!["XXH64".to_string()],
            source_verify: false,
            post_verify: true,
            generate_mhl: false,
            buffer_size: 8 * 1024 * 1024,
            max_retries: 1,
            cascade: false,
            default_dest_paths: Vec::new(),
            created_at: now.clone(),
            updated_at: now.clone(),
        },
        WorkflowPreset {
            id: "builtin-cascade".to_string(),
            name: "Cascade (SSD First)".to_string(),
            description: "Copy to fast SSD first, then cascade to shuttle drives. Frees source card sooner."
                .to_string(),
            hash_algorithms: vec!["XXH64".to_string(), "SHA256".to_string()],
            source_verify: true,
            post_verify: true,
            generate_mhl: true,
            buffer_size: 4 * 1024 * 1024,
            max_retries: 3,
            cascade: true,
            default_dest_paths: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
        },
    ]
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_load_empty_returns_default() {
        let tmp = TempDir::new().unwrap();
        let store = load_presets(tmp.path()).unwrap();
        assert!(store.presets.is_empty());
    }

    #[test]
    fn test_create_and_load_preset() {
        let tmp = TempDir::new().unwrap();
        let preset = WorkflowPreset {
            id: String::new(), // should be auto-assigned
            name: "My ARRI Preset".to_string(),
            description: "Day offload for ARRI Alexa Mini".to_string(),
            hash_algorithms: vec!["XXH64".to_string(), "SHA256".to_string()],
            source_verify: true,
            post_verify: true,
            generate_mhl: true,
            buffer_size: 4 * 1024 * 1024,
            max_retries: 3,
            cascade: false,
            default_dest_paths: vec!["/Volumes/SSD_A".to_string()],
            created_at: String::new(),
            updated_at: String::new(),
        };

        let created = create_preset(tmp.path(), preset).unwrap();
        assert!(!created.id.is_empty(), "ID should be auto-assigned");
        assert!(!created.created_at.is_empty());

        let store = load_presets(tmp.path()).unwrap();
        assert_eq!(store.presets.len(), 1);
        assert_eq!(store.presets[0].name, "My ARRI Preset");
        assert_eq!(store.presets[0].default_dest_paths, vec!["/Volumes/SSD_A"]);
    }

    #[test]
    fn test_update_preset() {
        let tmp = TempDir::new().unwrap();
        let preset = WorkflowPreset {
            id: "preset-1".to_string(),
            name: "Original Name".to_string(),
            description: String::new(),
            hash_algorithms: vec!["XXH64".to_string()],
            source_verify: true,
            post_verify: true,
            generate_mhl: true,
            buffer_size: 4 * 1024 * 1024,
            max_retries: 3,
            cascade: false,
            default_dest_paths: Vec::new(),
            created_at: String::new(),
            updated_at: String::new(),
        };

        create_preset(tmp.path(), preset).unwrap();

        let mut updated = load_presets(tmp.path()).unwrap().presets[0].clone();
        updated.name = "Updated Name".to_string();
        updated.cascade = true;

        let result = update_preset(tmp.path(), updated).unwrap();
        assert_eq!(result.name, "Updated Name");
        assert!(result.cascade);
        assert!(!result.created_at.is_empty());

        let store = load_presets(tmp.path()).unwrap();
        assert_eq!(store.presets.len(), 1);
        assert_eq!(store.presets[0].name, "Updated Name");
    }

    #[test]
    fn test_delete_preset() {
        let tmp = TempDir::new().unwrap();

        for name in &["Preset A", "Preset B", "Preset C"] {
            create_preset(
                tmp.path(),
                WorkflowPreset {
                    id: String::new(),
                    name: name.to_string(),
                    description: String::new(),
                    hash_algorithms: vec!["XXH64".to_string()],
                    source_verify: true,
                    post_verify: true,
                    generate_mhl: true,
                    buffer_size: 4 * 1024 * 1024,
                    max_retries: 3,
                    cascade: false,
                    default_dest_paths: Vec::new(),
                    created_at: String::new(),
                    updated_at: String::new(),
                },
            )
            .unwrap();
        }

        let store = load_presets(tmp.path()).unwrap();
        assert_eq!(store.presets.len(), 3);

        let id_to_delete = store.presets[1].id.clone();
        delete_preset(tmp.path(), &id_to_delete).unwrap();

        let store = load_presets(tmp.path()).unwrap();
        assert_eq!(store.presets.len(), 2);
        assert!(store.presets.iter().all(|p| p.id != id_to_delete));
    }

    #[test]
    fn test_delete_nonexistent_fails() {
        let tmp = TempDir::new().unwrap();
        let err = delete_preset(tmp.path(), "nonexistent").unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn test_builtin_presets() {
        let builtins = builtin_presets();
        assert_eq!(builtins.len(), 3);
        assert!(builtins.iter().any(|p| p.cascade));
        assert!(builtins.iter().any(|p| !p.source_verify));
    }

    #[test]
    fn test_serialization_roundtrip() {
        let store = PresetStore {
            presets: builtin_presets(),
        };
        let json = serde_json::to_string(&store).unwrap();
        let parsed: PresetStore = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.presets.len(), 3);
        assert_eq!(parsed.presets[0].name, store.presets[0].name);
    }
}
