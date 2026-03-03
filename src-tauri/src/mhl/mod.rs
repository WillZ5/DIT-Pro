//! ASC MHL (Media Hash List) — Chain of custody tracking.
//!
//! Implements the ASC MHL v2.0 standard for:
//! - History creation (first generation) and continuation (subsequent generations)
//! - Manifest XML writing (`<hashlist>` with `<creatorinfo>`, `<processinfo>`, `<hashes>`)
//! - Chain file management (`ascmhl_chain.xml`)
//! - Ignore patterns for system/temp files (.DS_Store, Thumbs.db, etc.)
//! - Creator info embedding (operator, software version, hostname)
//!
//! File layout:
//! ```text
//! MediaRoot/
//!   ascmhl/
//!     0001_MediaRoot_2024-01-15_120000Z.mhl
//!     0002_MediaRoot_2024-01-16_090000Z.mhl
//!     ascmhl_chain.xml
//!   Clips/
//!     A001C001.mov
//! ```

pub mod xml_writer;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::hash_engine::{HashAlgorithm, HashResult};

/// ASC MHL directory name (no leading dot per spec)
pub const ASCMHL_DIR_NAME: &str = "ascmhl";
/// Chain file name inside the ascmhl directory
pub const CHAIN_FILE_NAME: &str = "ascmhl_chain.xml";
/// ASC MHL v2.0 manifest namespace
pub const MHL_NAMESPACE: &str = "urn:ASC:MHL:v2.0";
/// ASC MHL v2.0 chain namespace
pub const CHAIN_NAMESPACE: &str = "urn:ASC:MHL:DIRECTORY:v2.0";

/// Default ignore patterns for files that should not be hashed
pub const DEFAULT_IGNORE_PATTERNS: &[&str] = &[
    ".DS_Store",
    "Thumbs.db",
    "desktop.ini",
    "*.tmp",
    "._*",
    "ascmhl",
    "ascmhl/",
];

/// Creator information embedded in MHL records
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MhlCreatorInfo {
    pub tool_name: String,
    pub tool_version: String,
    pub hostname: Option<String>,
    pub location: Option<String>,
    pub comment: Option<String>,
    pub authors: Vec<MhlAuthor>,
}

/// Author (operator) information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MhlAuthor {
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub role: Option<String>,
}

/// Configuration for MHL generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MhlConfig {
    pub creator_info: MhlCreatorInfo,
    /// File patterns to ignore (e.g., ".DS_Store", "*.tmp")
    pub ignore_patterns: Vec<String>,
    /// Hash algorithm used for file entries (default: XXH64)
    pub hash_format: HashAlgorithm,
}

impl Default for MhlConfig {
    fn default() -> Self {
        Self {
            creator_info: MhlCreatorInfo {
                tool_name: "DIT Pro".to_string(),
                tool_version: env!("CARGO_PKG_VERSION").to_string(),
                hostname: get_hostname(),
                location: None,
                comment: None,
                authors: Vec::new(),
            },
            ignore_patterns: DEFAULT_IGNORE_PATTERNS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            hash_format: HashAlgorithm::XXH64,
        }
    }
}

/// Process type for this generation
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum MhlProcessType {
    /// Files hashed at current location
    InPlace,
    /// Files were copied/transferred
    Transfer,
    /// History flattened into external manifest
    Flatten,
}

impl std::fmt::Display for MhlProcessType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MhlProcessType::InPlace => write!(f, "in-place"),
            MhlProcessType::Transfer => write!(f, "transfer"),
            MhlProcessType::Flatten => write!(f, "flatten"),
        }
    }
}

/// Hash action for a file entry
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum MhlHashAction {
    /// First time this file has been hashed
    Original,
    /// Hash matches a previously recorded hash
    Verified,
    /// Hash does NOT match (corruption detected)
    Failed,
}

impl std::fmt::Display for MhlHashAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MhlHashAction::Original => write!(f, "original"),
            MhlHashAction::Verified => write!(f, "verified"),
            MhlHashAction::Failed => write!(f, "failed"),
        }
    }
}

/// A single file hash entry in the manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MhlHashEntry {
    /// Relative path from media root (POSIX separators)
    pub path: String,
    /// File size in bytes
    pub file_size: u64,
    /// File last modification date
    pub last_modified: DateTime<Utc>,
    /// Hash results (one per algorithm)
    pub hashes: Vec<MhlFileHash>,
}

/// A hash value for a specific algorithm within a file entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MhlFileHash {
    pub algorithm: HashAlgorithm,
    pub hex_digest: String,
    pub action: MhlHashAction,
    pub hash_date: DateTime<Utc>,
}

/// A directory hash entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MhlDirectoryHashEntry {
    /// Relative path from media root
    pub path: String,
    pub last_modified: DateTime<Utc>,
    /// Hash of combined file content hashes
    pub content_hash: String,
    /// Hash of directory structure (filenames)
    pub structure_hash: String,
    pub hash_algorithm: HashAlgorithm,
    pub hash_date: DateTime<Utc>,
}

/// A complete MHL generation (manifest)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MhlGeneration {
    /// Generation number (1-based)
    pub generation: u32,
    /// Creation timestamp
    pub creation_date: DateTime<Utc>,
    /// Creator information
    pub creator_info: MhlCreatorInfo,
    /// Process type
    pub process_type: MhlProcessType,
    /// Root hash (content + structure)
    pub root_content_hash: Option<String>,
    pub root_structure_hash: Option<String>,
    /// Ignore patterns
    pub ignore_patterns: Vec<String>,
    /// File hash entries
    pub hash_entries: Vec<MhlHashEntry>,
    /// Directory hash entries
    pub directory_hashes: Vec<MhlDirectoryHashEntry>,
    /// Hash algorithm used
    pub hash_algorithm: HashAlgorithm,
}

/// Entry in the chain file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MhlChainEntry {
    pub sequence_nr: u32,
    pub path: String,
    /// SHA-256 hash of the manifest file (spec uses C4, we use SHA-256 for MVP)
    pub reference_hash: String,
}

/// An MHL history (the ascmhl directory state)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MhlHistory {
    /// Root directory being tracked
    pub root_path: PathBuf,
    /// Name of the root directory (used in filenames)
    pub root_name: String,
    /// Chain entries (one per generation)
    pub chain: Vec<MhlChainEntry>,
}

impl MhlHistory {
    /// Get the path to the ascmhl directory
    pub fn ascmhl_dir(&self) -> PathBuf {
        self.root_path.join(ASCMHL_DIR_NAME)
    }

    /// Get the next generation number
    pub fn next_generation(&self) -> u32 {
        self.chain.last().map(|e| e.sequence_nr + 1).unwrap_or(1)
    }

    /// Generate the filename for a generation
    pub fn generation_filename(
        root_name: &str,
        generation: u32,
        timestamp: &DateTime<Utc>,
    ) -> String {
        format!(
            "{:04}_{}_{}Z.mhl",
            generation,
            root_name,
            timestamp.format("%Y-%m-%d_%H%M%S")
        )
    }
}

/// Create or load an MHL history for a root directory.
///
/// If the ascmhl directory exists, loads the chain file.
/// If not, returns a fresh history ready for the first generation.
pub async fn load_or_create_history(root_path: &Path) -> Result<MhlHistory> {
    let root_name = root_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("media")
        .to_string();

    let ascmhl_dir = root_path.join(ASCMHL_DIR_NAME);

    if ascmhl_dir.exists() {
        // Load existing chain
        let chain_path = ascmhl_dir.join(CHAIN_FILE_NAME);
        if chain_path.exists() {
            let chain = xml_writer::parse_chain_file(&chain_path).await?;
            Ok(MhlHistory {
                root_path: root_path.to_path_buf(),
                root_name,
                chain,
            })
        } else {
            // ascmhl dir exists but no chain file — treat as fresh
            Ok(MhlHistory {
                root_path: root_path.to_path_buf(),
                root_name,
                chain: Vec::new(),
            })
        }
    } else {
        Ok(MhlHistory {
            root_path: root_path.to_path_buf(),
            root_name,
            chain: Vec::new(),
        })
    }
}

/// Create a new MHL generation for file hash results.
///
/// This is the main entry point after a copy/verify operation:
/// 1. Collects file hash results
/// 2. Computes root content/structure hashes
/// 3. Writes the manifest XML
/// 4. Updates the chain file
///
/// Returns the path to the new manifest file.
pub async fn create_generation(
    history: &mut MhlHistory,
    file_hashes: &HashMap<String, Vec<HashResult>>,
    file_metadata: &HashMap<String, (u64, DateTime<Utc>)>,
    process_type: MhlProcessType,
    config: &MhlConfig,
) -> Result<PathBuf> {
    let now = Utc::now();
    let generation_num = history.next_generation();
    let action = if generation_num == 1 {
        MhlHashAction::Original
    } else {
        MhlHashAction::Verified
    };

    // Build hash entries
    let mut hash_entries: Vec<MhlHashEntry> = Vec::new();
    let mut sorted_paths: Vec<&String> = file_hashes.keys().collect();
    sorted_paths.sort();

    for rel_path in sorted_paths {
        let results = &file_hashes[rel_path];
        let (file_size, last_modified) = file_metadata
            .get(rel_path.as_str())
            .copied()
            .unwrap_or((0, now));

        let hashes: Vec<MhlFileHash> = results
            .iter()
            .map(|r| MhlFileHash {
                algorithm: r.algorithm,
                hex_digest: r.hex_digest.clone(),
                action,
                hash_date: now,
            })
            .collect();

        hash_entries.push(MhlHashEntry {
            path: rel_path.clone(),
            file_size,
            last_modified,
            hashes,
        });
    }

    // Compute root content hash (hash of all file content hashes concatenated)
    let root_content_hash = compute_root_content_hash(&hash_entries);
    // Compute root structure hash (hash of all relative paths concatenated)
    let root_structure_hash = compute_root_structure_hash(&hash_entries);

    // Build generation
    let generation = MhlGeneration {
        generation: generation_num,
        creation_date: now,
        creator_info: config.creator_info.clone(),
        process_type,
        root_content_hash: Some(root_content_hash),
        root_structure_hash: Some(root_structure_hash),
        ignore_patterns: config.ignore_patterns.clone(),
        hash_entries,
        directory_hashes: Vec::new(), // Directory hashes computed separately if needed
        hash_algorithm: config.hash_format,
    };

    // Ensure ascmhl directory exists
    let ascmhl_dir = history.ascmhl_dir();
    tokio::fs::create_dir_all(&ascmhl_dir)
        .await
        .context("Failed to create ascmhl directory")?;

    // Write the manifest XML
    let filename = MhlHistory::generation_filename(&history.root_name, generation_num, &now);
    let manifest_path = ascmhl_dir.join(&filename);
    xml_writer::write_manifest(&manifest_path, &generation).await?;

    // Compute SHA-256 of the manifest file for the chain
    let manifest_bytes = tokio::fs::read(&manifest_path).await?;
    let mut sha = Sha256::new();
    sha.update(&manifest_bytes);
    let reference_hash = format!("{:x}", sha.finalize());

    // Update chain
    let chain_entry = MhlChainEntry {
        sequence_nr: generation_num,
        path: filename,
        reference_hash,
    };
    history.chain.push(chain_entry);

    // Write chain file
    let chain_path = ascmhl_dir.join(CHAIN_FILE_NAME);
    xml_writer::write_chain_file(&chain_path, &history.chain).await?;

    Ok(manifest_path)
}

/// Check if a relative path should be ignored based on patterns
pub fn should_ignore(rel_path: &str, patterns: &[String]) -> bool {
    let filename = Path::new(rel_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    for pattern in patterns {
        // Exact filename match
        if filename == *pattern {
            return true;
        }
        // Directory match (pattern ends with /)
        if pattern.ends_with('/') {
            let dir_name = &pattern[..pattern.len() - 1];
            if rel_path.starts_with(dir_name)
                || rel_path.contains(&format!("/{}/", dir_name))
                || rel_path == dir_name
            {
                return true;
            }
        }
        // Wildcard match: *.ext (suffix) or prefix.* (prefix)
        if let Some(suffix) = pattern.strip_prefix('*') {
            if filename.ends_with(suffix) {
                return true;
            }
        }
        if let Some(prefix) = pattern.strip_suffix('*') {
            if filename.starts_with(prefix) {
                return true;
            }
        }
    }
    false
}

/// Compute a root content hash from all file hash entries.
/// This is a hash of all primary file hashes concatenated, sorted by path.
fn compute_root_content_hash(entries: &[MhlHashEntry]) -> String {
    let mut hasher = Sha256::new();
    for entry in entries {
        if let Some(first_hash) = entry.hashes.first() {
            hasher.update(first_hash.hex_digest.as_bytes());
        }
    }
    format!("{:x}", hasher.finalize())
}

/// Compute a root structure hash from all file paths.
/// This detects renames, additions, deletions.
fn compute_root_structure_hash(entries: &[MhlHashEntry]) -> String {
    let mut hasher = Sha256::new();
    for entry in entries {
        hasher.update(entry.path.as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

/// Get the system hostname
fn get_hostname() -> Option<String> {
    #[cfg(unix)]
    {
        use std::process::Command;
        Command::new("hostname")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Verify a manifest file's integrity against its chain entry
pub async fn verify_chain_entry(ascmhl_dir: &Path, entry: &MhlChainEntry) -> Result<bool> {
    let manifest_path = ascmhl_dir.join(&entry.path);
    if !manifest_path.exists() {
        bail!("Manifest file not found: {:?}", manifest_path);
    }

    let manifest_bytes = tokio::fs::read(&manifest_path).await?;
    let mut sha = Sha256::new();
    sha.update(&manifest_bytes);
    let computed_hash = format!("{:x}", sha.finalize());

    Ok(computed_hash == entry.reference_hash)
}

/// Verify all chain entries in a history
pub async fn verify_chain(history: &MhlHistory) -> Result<Vec<(u32, bool)>> {
    let ascmhl_dir = history.ascmhl_dir();
    let mut results = Vec::new();

    for entry in &history.chain {
        let valid = verify_chain_entry(&ascmhl_dir, entry).await?;
        results.push((entry.sequence_nr, valid));
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_ignore_ds_store() {
        let patterns: Vec<String> = DEFAULT_IGNORE_PATTERNS
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(should_ignore(".DS_Store", &patterns));
        assert!(should_ignore("Clips/.DS_Store", &patterns));
    }

    #[test]
    fn test_should_ignore_tmp_wildcard() {
        let patterns: Vec<String> = DEFAULT_IGNORE_PATTERNS
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(should_ignore("file.tmp", &patterns));
        assert!(should_ignore("subdir/data.tmp", &patterns));
    }

    #[test]
    fn test_should_ignore_ascmhl_dir() {
        let patterns: Vec<String> = DEFAULT_IGNORE_PATTERNS
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(should_ignore("ascmhl/0001_test.mhl", &patterns));
        assert!(should_ignore("ascmhl", &patterns));
    }

    #[test]
    fn test_should_not_ignore_normal_files() {
        let patterns: Vec<String> = DEFAULT_IGNORE_PATTERNS
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(!should_ignore("Clips/A001C001.mov", &patterns));
        assert!(!should_ignore("Sidecar.txt", &patterns));
        assert!(!should_ignore("subfolder/data.r3d", &patterns));
    }

    #[test]
    fn test_generation_filename() {
        let ts = chrono::DateTime::parse_from_rfc3339("2024-01-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let filename = MhlHistory::generation_filename("A002R2EC", 1, &ts);
        assert_eq!(filename, "0001_A002R2EC_2024-01-15_120000Z.mhl");
    }

    #[test]
    fn test_generation_filename_multi_digit() {
        let ts = chrono::DateTime::parse_from_rfc3339("2024-12-31T23:59:59Z")
            .unwrap()
            .with_timezone(&Utc);
        let filename = MhlHistory::generation_filename("Card_A", 42, &ts);
        assert_eq!(filename, "0042_Card_A_2024-12-31_235959Z.mhl");
    }

    #[test]
    fn test_next_generation_empty() {
        let history = MhlHistory {
            root_path: PathBuf::from("/tmp/test"),
            root_name: "test".to_string(),
            chain: Vec::new(),
        };
        assert_eq!(history.next_generation(), 1);
    }

    #[test]
    fn test_next_generation_with_entries() {
        let history = MhlHistory {
            root_path: PathBuf::from("/tmp/test"),
            root_name: "test".to_string(),
            chain: vec![
                MhlChainEntry {
                    sequence_nr: 1,
                    path: "0001.mhl".to_string(),
                    reference_hash: "abc".to_string(),
                },
                MhlChainEntry {
                    sequence_nr: 2,
                    path: "0002.mhl".to_string(),
                    reference_hash: "def".to_string(),
                },
            ],
        };
        assert_eq!(history.next_generation(), 3);
    }

    #[test]
    fn test_root_content_hash_deterministic() {
        let entries = vec![
            MhlHashEntry {
                path: "a.mov".to_string(),
                file_size: 100,
                last_modified: Utc::now(),
                hashes: vec![MhlFileHash {
                    algorithm: HashAlgorithm::XXH64,
                    hex_digest: "abc123".to_string(),
                    action: MhlHashAction::Original,
                    hash_date: Utc::now(),
                }],
            },
            MhlHashEntry {
                path: "b.mov".to_string(),
                file_size: 200,
                last_modified: Utc::now(),
                hashes: vec![MhlFileHash {
                    algorithm: HashAlgorithm::XXH64,
                    hex_digest: "def456".to_string(),
                    action: MhlHashAction::Original,
                    hash_date: Utc::now(),
                }],
            },
        ];

        let h1 = compute_root_content_hash(&entries);
        let h2 = compute_root_content_hash(&entries);
        assert_eq!(h1, h2);
        assert!(!h1.is_empty());
    }

    #[test]
    fn test_root_structure_hash_changes_with_paths() {
        let now = Utc::now();
        let entries_a = vec![MhlHashEntry {
            path: "a.mov".to_string(),
            file_size: 100,
            last_modified: now,
            hashes: vec![],
        }];
        let entries_b = vec![MhlHashEntry {
            path: "b.mov".to_string(),
            file_size: 100,
            last_modified: now,
            hashes: vec![],
        }];

        let h1 = compute_root_structure_hash(&entries_a);
        let h2 = compute_root_structure_hash(&entries_b);
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_default_config() {
        let config = MhlConfig::default();
        assert_eq!(config.creator_info.tool_name, "DIT Pro");
        assert!(!config.ignore_patterns.is_empty());
        assert_eq!(config.hash_format, HashAlgorithm::XXH64);
    }

    #[tokio::test]
    async fn test_create_and_verify_generation() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("MediaCard");
        tokio::fs::create_dir_all(&root).await.unwrap();

        // Create some test files
        tokio::fs::write(root.join("clip1.mov"), b"video data 1")
            .await
            .unwrap();
        tokio::fs::write(root.join("clip2.mov"), b"video data 2")
            .await
            .unwrap();

        let mut history = load_or_create_history(&root).await.unwrap();
        assert_eq!(history.next_generation(), 1);

        // Prepare file hashes (as if from copy engine)
        let mut file_hashes = HashMap::new();
        file_hashes.insert(
            "clip1.mov".to_string(),
            vec![HashResult {
                algorithm: HashAlgorithm::XXH64,
                hex_digest: "abc123def456".to_string(),
            }],
        );
        file_hashes.insert(
            "clip2.mov".to_string(),
            vec![HashResult {
                algorithm: HashAlgorithm::XXH64,
                hex_digest: "789xyz000111".to_string(),
            }],
        );

        let now = Utc::now();
        let mut file_metadata = HashMap::new();
        file_metadata.insert("clip1.mov".to_string(), (12u64, now));
        file_metadata.insert("clip2.mov".to_string(), (12u64, now));

        let config = MhlConfig::default();

        // Create first generation
        let manifest_path = create_generation(
            &mut history,
            &file_hashes,
            &file_metadata,
            MhlProcessType::Transfer,
            &config,
        )
        .await
        .unwrap();

        assert!(manifest_path.exists());
        assert_eq!(history.chain.len(), 1);
        assert_eq!(history.chain[0].sequence_nr, 1);

        // Verify chain integrity
        let chain_results = verify_chain(&history).await.unwrap();
        assert_eq!(chain_results.len(), 1);
        assert!(chain_results[0].1); // should be valid

        // Verify ascmhl directory structure
        let ascmhl_dir = root.join(ASCMHL_DIR_NAME);
        assert!(ascmhl_dir.exists());
        assert!(ascmhl_dir.join(CHAIN_FILE_NAME).exists());

        // Create second generation
        let manifest_path_2 = create_generation(
            &mut history,
            &file_hashes,
            &file_metadata,
            MhlProcessType::InPlace,
            &config,
        )
        .await
        .unwrap();

        assert!(manifest_path_2.exists());
        assert_eq!(history.chain.len(), 2);
        assert_eq!(history.chain[1].sequence_nr, 2);

        // Verify both chain entries
        let chain_results_2 = verify_chain(&history).await.unwrap();
        assert_eq!(chain_results_2.len(), 2);
        assert!(chain_results_2[0].1);
        assert!(chain_results_2[1].1);
    }

    #[tokio::test]
    async fn test_load_existing_history() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("TestMedia");
        tokio::fs::create_dir_all(&root).await.unwrap();

        // Create a history with one generation
        let mut history = load_or_create_history(&root).await.unwrap();
        let mut file_hashes = HashMap::new();
        file_hashes.insert(
            "test.mov".to_string(),
            vec![HashResult {
                algorithm: HashAlgorithm::XXH64,
                hex_digest: "aabbccdd".to_string(),
            }],
        );
        let mut file_metadata = HashMap::new();
        file_metadata.insert("test.mov".to_string(), (8u64, Utc::now()));

        create_generation(
            &mut history,
            &file_hashes,
            &file_metadata,
            MhlProcessType::Transfer,
            &MhlConfig::default(),
        )
        .await
        .unwrap();

        // Reload the history from disk
        let loaded = load_or_create_history(&root).await.unwrap();
        assert_eq!(loaded.chain.len(), 1);
        assert_eq!(loaded.chain[0].sequence_nr, 1);
        assert_eq!(loaded.next_generation(), 2);
    }

    #[tokio::test]
    async fn test_chain_tamper_detection() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("SecureMedia");
        tokio::fs::create_dir_all(&root).await.unwrap();

        let mut history = load_or_create_history(&root).await.unwrap();
        let mut file_hashes = HashMap::new();
        file_hashes.insert(
            "clip.mov".to_string(),
            vec![HashResult {
                algorithm: HashAlgorithm::XXH64,
                hex_digest: "feedface".to_string(),
            }],
        );
        let mut file_metadata = HashMap::new();
        file_metadata.insert("clip.mov".to_string(), (4u64, Utc::now()));

        let manifest_path = create_generation(
            &mut history,
            &file_hashes,
            &file_metadata,
            MhlProcessType::Transfer,
            &MhlConfig::default(),
        )
        .await
        .unwrap();

        // Tamper with the manifest
        let mut content = tokio::fs::read_to_string(&manifest_path).await.unwrap();
        content = content.replace("feedface", "deadbeef");
        tokio::fs::write(&manifest_path, content).await.unwrap();

        // Chain verification should detect tampering
        let results = verify_chain(&history).await.unwrap();
        assert!(!results[0].1); // should fail
    }
}
