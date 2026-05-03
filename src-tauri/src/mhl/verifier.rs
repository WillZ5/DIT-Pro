//! Application-level ASC MHL verification.
//!
//! Verifies both the `ascmhl_chain.xml` chain references and, unless requested
//! otherwise, recomputes media file hashes from MHL manifests.

use anyhow::{bail, Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use crate::hash_engine::{self, HashAlgorithm, HashEngineConfig};

use super::{ASCMHL_DIR_NAME, CHAIN_FILE_NAME};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MhlVerifyOptions {
    #[serde(default)]
    pub chain_only: bool,
    #[serde(default)]
    pub verify_all_generations: bool,
    #[serde(default)]
    pub generation: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MhlVerifyReport {
    pub summary: MhlVerifySummary,
    pub chain_results: Vec<MhlChainCheckResult>,
    pub issues: Vec<MhlVerifyIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MhlVerifySummary {
    pub path: String,
    pub mode: String,
    pub success: bool,
    pub chain_only: bool,
    pub chain_entries: usize,
    pub chain_valid: usize,
    pub chain_invalid: usize,
    pub total_files: usize,
    pub passed: usize,
    pub failed: usize,
    pub missing: usize,
    pub errors: usize,
    pub verified_generations: Vec<u32>,
    pub duration_secs: f64,
}

impl MhlVerifySummary {
    fn new(path: &Path, mode: &str, chain_only: bool) -> Self {
        Self {
            path: path.to_string_lossy().to_string(),
            mode: mode.to_string(),
            success: false,
            chain_only,
            chain_entries: 0,
            chain_valid: 0,
            chain_invalid: 0,
            total_files: 0,
            passed: 0,
            failed: 0,
            missing: 0,
            errors: 0,
            verified_generations: Vec::new(),
            duration_secs: 0.0,
        }
    }

    fn finalize(&mut self, elapsed: f64) {
        self.duration_secs = elapsed;
        self.success =
            self.chain_invalid == 0 && self.failed == 0 && self.missing == 0 && self.errors == 0;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MhlChainCheckResult {
    pub generation: u32,
    pub manifest_path: String,
    pub expected_hash: String,
    pub actual_hash: Option<String>,
    pub valid: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MhlVerifyIssue {
    pub kind: String,
    pub message: String,
    pub generation: Option<u32>,
    pub rel_path: Option<String>,
    pub manifest_path: Option<String>,
    pub algorithm: Option<HashAlgorithm>,
    pub expected: Option<String>,
    pub actual: Option<String>,
}

#[derive(Debug, Clone)]
struct ManifestFileEntry {
    path: String,
    hashes: HashMap<HashAlgorithm, String>,
}

#[derive(Debug, Clone)]
struct ChainEntry {
    sequence_nr: u32,
    path: String,
    reference_hash: String,
}

pub fn verify_mhl_path(path: &Path, options: MhlVerifyOptions) -> Result<MhlVerifyReport> {
    if !path.exists() {
        bail!("Path does not exist: {}", path.display());
    }

    let started = Instant::now();
    let mut report = if is_mhl_file(path) {
        verify_single_manifest(path, &options)?
    } else if path.is_dir() {
        verify_directory(path, &options)?
    } else {
        bail!(
            "Path must be a directory containing ascmhl/ or a .mhl file: {}",
            path.display()
        );
    };

    report.summary.finalize(started.elapsed().as_secs_f64());
    Ok(report)
}

fn verify_directory(root: &Path, options: &MhlVerifyOptions) -> Result<MhlVerifyReport> {
    let ascmhl_dir = root.join(ASCMHL_DIR_NAME);
    if !ascmhl_dir.exists() {
        bail!("No ascmhl/ directory found in: {}", root.display());
    }

    let chain_path = ascmhl_dir.join(CHAIN_FILE_NAME);
    if !chain_path.exists() {
        bail!("No ascmhl_chain.xml found in: {}", ascmhl_dir.display());
    }

    let chain = parse_chain_file(&chain_path)?;
    if chain.is_empty() {
        bail!("Chain file is empty: {}", chain_path.display());
    }

    let mut summary = MhlVerifySummary::new(root, "directory", options.chain_only);
    summary.chain_entries = chain.len();
    let mut issues = Vec::new();
    let mut chain_results = Vec::new();

    for entry in &chain {
        let manifest_path = ascmhl_dir.join(&entry.path);
        let result = verify_chain_entry(&manifest_path, &entry.reference_hash);
        match result {
            Ok((valid, actual_hash)) => {
                if valid {
                    summary.chain_valid += 1;
                } else {
                    summary.chain_invalid += 1;
                    issues.push(MhlVerifyIssue {
                        kind: "chainMismatch".to_string(),
                        message: format!(
                            "Generation {:04} manifest hash does not match chain reference",
                            entry.sequence_nr
                        ),
                        generation: Some(entry.sequence_nr),
                        rel_path: None,
                        manifest_path: Some(manifest_path.to_string_lossy().to_string()),
                        algorithm: Some(HashAlgorithm::SHA256),
                        expected: Some(entry.reference_hash.clone()),
                        actual: Some(actual_hash.clone()),
                    });
                }
                chain_results.push(MhlChainCheckResult {
                    generation: entry.sequence_nr,
                    manifest_path: manifest_path.to_string_lossy().to_string(),
                    expected_hash: entry.reference_hash.clone(),
                    actual_hash: Some(actual_hash),
                    valid,
                    error: None,
                });
            }
            Err(e) => {
                summary.chain_invalid += 1;
                let message = e.to_string();
                issues.push(MhlVerifyIssue {
                    kind: "chainError".to_string(),
                    message: message.clone(),
                    generation: Some(entry.sequence_nr),
                    rel_path: None,
                    manifest_path: Some(manifest_path.to_string_lossy().to_string()),
                    algorithm: Some(HashAlgorithm::SHA256),
                    expected: Some(entry.reference_hash.clone()),
                    actual: None,
                });
                chain_results.push(MhlChainCheckResult {
                    generation: entry.sequence_nr,
                    manifest_path: manifest_path.to_string_lossy().to_string(),
                    expected_hash: entry.reference_hash.clone(),
                    actual_hash: None,
                    valid: false,
                    error: Some(message),
                });
            }
        }
    }

    if !options.chain_only {
        let generations = select_generations(&chain, options)?;
        for entry in generations {
            let manifest_path = ascmhl_dir.join(&entry.path);
            let manifest_entries = parse_manifest_file(&manifest_path)?;
            summary.verified_generations.push(entry.sequence_nr);
            verify_manifest_entries(
                root,
                &manifest_path,
                entry.sequence_nr,
                &manifest_entries,
                &mut summary,
                &mut issues,
            );
        }
    }

    Ok(MhlVerifyReport {
        summary,
        chain_results,
        issues,
    })
}

fn verify_single_manifest(
    manifest_path: &Path,
    options: &MhlVerifyOptions,
) -> Result<MhlVerifyReport> {
    let root = manifest_path
        .parent()
        .and_then(|p| {
            if p.file_name().is_some_and(|n| n == ASCMHL_DIR_NAME) {
                p.parent()
            } else {
                Some(p)
            }
        })
        .unwrap_or_else(|| Path::new("."));

    let mut summary = MhlVerifySummary::new(manifest_path, "manifest", options.chain_only);
    let mut issues = Vec::new();

    let entries = parse_manifest_file(manifest_path)?;

    if options.chain_only {
        summary.total_files = entries.len();
    } else {
        verify_manifest_entries(root, manifest_path, 0, &entries, &mut summary, &mut issues);
    }

    Ok(MhlVerifyReport {
        summary,
        chain_results: Vec::new(),
        issues,
    })
}

fn select_generations<'a>(
    chain: &'a [ChainEntry],
    options: &MhlVerifyOptions,
) -> Result<Vec<&'a ChainEntry>> {
    if let Some(generation) = options.generation {
        let selected: Vec<&ChainEntry> = chain
            .iter()
            .filter(|entry| entry.sequence_nr == generation)
            .collect();
        if selected.is_empty() {
            bail!("Generation {} not found in MHL chain", generation);
        }
        Ok(selected)
    } else if options.verify_all_generations {
        Ok(chain.iter().collect())
    } else {
        Ok(chain.last().into_iter().collect())
    }
}

fn verify_manifest_entries(
    root: &Path,
    manifest_path: &Path,
    generation: u32,
    entries: &[ManifestFileEntry],
    summary: &mut MhlVerifySummary,
    issues: &mut Vec<MhlVerifyIssue>,
) {
    summary.total_files += entries.len();

    for entry in entries {
        let file_path = match resolve_manifest_file_path(root, &entry.path) {
            Ok(path) => path,
            Err(e) => {
                summary.errors += 1;
                issues.push(MhlVerifyIssue {
                    kind: "unsafePath".to_string(),
                    message: e.to_string(),
                    generation: generation_option(generation),
                    rel_path: Some(entry.path.clone()),
                    manifest_path: Some(manifest_path.to_string_lossy().to_string()),
                    algorithm: None,
                    expected: None,
                    actual: None,
                });
                continue;
            }
        };
        if !file_path.exists() {
            summary.missing += 1;
            issues.push(MhlVerifyIssue {
                kind: "missing".to_string(),
                message: format!("File not found: {}", entry.path),
                generation: generation_option(generation),
                rel_path: Some(entry.path.clone()),
                manifest_path: Some(manifest_path.to_string_lossy().to_string()),
                algorithm: None,
                expected: None,
                actual: None,
            });
            continue;
        }

        let algorithms: Vec<HashAlgorithm> = entry.hashes.keys().copied().collect();
        let config = HashEngineConfig {
            algorithms,
            buffer_size: 4 * 1024 * 1024,
        };

        let hash_results = match hash_engine::hash_file_sync(&file_path, &config) {
            Ok(results) => results,
            Err(e) => {
                summary.errors += 1;
                issues.push(MhlVerifyIssue {
                    kind: "readError".to_string(),
                    message: format!("Could not read {}: {}", entry.path, e),
                    generation: generation_option(generation),
                    rel_path: Some(entry.path.clone()),
                    manifest_path: Some(manifest_path.to_string_lossy().to_string()),
                    algorithm: None,
                    expected: None,
                    actual: None,
                });
                continue;
            }
        };

        let actual: HashMap<HashAlgorithm, String> = hash_results
            .into_iter()
            .map(|result| (result.algorithm, result.hex_digest))
            .collect();

        let mut file_failed = false;
        for (algorithm, expected_hash) in &entry.hashes {
            let actual_hash = actual.get(algorithm).cloned().unwrap_or_default();
            if actual_hash.to_lowercase() != expected_hash.to_lowercase() {
                file_failed = true;
                issues.push(MhlVerifyIssue {
                    kind: "hashMismatch".to_string(),
                    message: format!("Hash mismatch for {}", entry.path),
                    generation: generation_option(generation),
                    rel_path: Some(entry.path.clone()),
                    manifest_path: Some(manifest_path.to_string_lossy().to_string()),
                    algorithm: Some(*algorithm),
                    expected: Some(expected_hash.clone()),
                    actual: Some(actual_hash),
                });
            }
        }

        if file_failed {
            summary.failed += 1;
        } else {
            summary.passed += 1;
        }
    }
}

fn resolve_manifest_file_path(root: &Path, rel_path: &str) -> Result<PathBuf> {
    let path = Path::new(rel_path);
    if path.is_absolute() {
        bail!("Manifest path must be relative: {}", rel_path);
    }

    let mut safe_path = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => safe_path.push(part),
            Component::CurDir => {}
            _ => bail!("Manifest path escapes media root: {}", rel_path),
        }
    }

    if safe_path.as_os_str().is_empty() {
        bail!("Manifest path is empty");
    }

    Ok(root.join(safe_path))
}

fn generation_option(generation: u32) -> Option<u32> {
    if generation == 0 {
        None
    } else {
        Some(generation)
    }
}

fn is_mhl_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mhl"))
}

fn verify_chain_entry(manifest_path: &Path, expected_hash: &str) -> Result<(bool, String)> {
    let bytes = fs::read(manifest_path)
        .with_context(|| format!("Failed to read manifest: {}", manifest_path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let computed = format!("{:x}", hasher.finalize());
    Ok((computed.eq_ignore_ascii_case(expected_hash), computed))
}

fn parse_chain_file(path: &Path) -> Result<Vec<ChainEntry>> {
    let content =
        fs::read(path).with_context(|| format!("Failed to read chain file: {}", path.display()))?;
    parse_chain_xml(&content)
}

fn parse_chain_xml(xml_bytes: &[u8]) -> Result<Vec<ChainEntry>> {
    let mut reader = Reader::from_reader(xml_bytes);
    reader.trim_text(true);

    let mut entries = Vec::new();
    let mut current_entry: Option<ChainEntry> = None;
    let mut current_element = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                match name.as_str() {
                    "hashlist" => {
                        let mut sequence_nr = 0;
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"sequencenr" {
                                sequence_nr =
                                    String::from_utf8_lossy(&attr.value).parse().unwrap_or(0);
                            }
                        }
                        current_entry = Some(ChainEntry {
                            sequence_nr,
                            path: String::new(),
                            reference_hash: String::new(),
                        });
                    }
                    "path" | "sha256" | "c4" => {
                        current_element = name;
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref e)) => {
                let text = e.unescape().unwrap_or_default().to_string();
                if let Some(ref mut entry) = current_entry {
                    match current_element.as_str() {
                        "path" => entry.path = text,
                        "sha256" | "c4" => entry.reference_hash = text,
                        _ => {}
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                if name == "hashlist" {
                    if let Some(entry) = current_entry.take() {
                        entries.push(entry);
                    }
                }
                current_element.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                bail!(
                    "Error parsing chain XML at position {}: {:?}",
                    reader.buffer_position(),
                    e
                );
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(entries)
}

fn parse_manifest_file(path: &Path) -> Result<Vec<ManifestFileEntry>> {
    let content =
        fs::read(path).with_context(|| format!("Failed to read manifest: {}", path.display()))?;
    parse_manifest_xml(&content)
}

fn parse_manifest_xml(xml_bytes: &[u8]) -> Result<Vec<ManifestFileEntry>> {
    let mut reader = Reader::from_reader(xml_bytes);
    reader.trim_text(true);

    let mut entries = Vec::new();
    let mut buf = Vec::new();
    let mut in_hash_block = false;
    let mut in_hashes_section = false;
    let mut current_file_path = String::new();
    let mut current_hashes: HashMap<HashAlgorithm, String> = HashMap::new();
    let mut current_element = String::new();
    let mut current_algo: Option<HashAlgorithm> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                match name.as_str() {
                    "hashes" => in_hashes_section = true,
                    "hash" if in_hashes_section => {
                        in_hash_block = true;
                        current_file_path.clear();
                        current_hashes.clear();
                    }
                    "path" if in_hash_block => current_element = "path".to_string(),
                    algo_name if in_hash_block => {
                        if let Some(algo) = hash_algorithm_from_xml_name(algo_name) {
                            current_algo = Some(algo);
                            current_element = algo_name.to_string();
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref e)) => {
                let text = e.unescape().unwrap_or_default().to_string();
                if current_element == "path" && in_hash_block {
                    current_file_path = text;
                } else if let Some(algo) = current_algo {
                    current_hashes.insert(algo, text);
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                match name.as_str() {
                    "hash" if in_hash_block => {
                        if !current_file_path.is_empty() && !current_hashes.is_empty() {
                            entries.push(ManifestFileEntry {
                                path: current_file_path.clone(),
                                hashes: current_hashes.clone(),
                            });
                        }
                        in_hash_block = false;
                    }
                    "hashes" => in_hashes_section = false,
                    _ => {}
                }
                current_element.clear();
                current_algo = None;
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                bail!(
                    "Error parsing manifest XML at position {}: {:?}",
                    reader.buffer_position(),
                    e
                );
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(entries)
}

fn hash_algorithm_from_xml_name(name: &str) -> Option<HashAlgorithm> {
    match name.to_lowercase().as_str() {
        "xxh64" => Some(HashAlgorithm::XXH64),
        "xxh3" => Some(HashAlgorithm::XXH3),
        "xxh128" => Some(HashAlgorithm::XXH128),
        "sha256" | "sha-256" => Some(HashAlgorithm::SHA256),
        "md5" => Some(HashAlgorithm::MD5),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest_xml(hash: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<hashlist version="2.0" xmlns="urn:ASC:MHL:v2.0">
  <hashes>
    <hash>
      <path size="18">Clips/test.mov</path>
      <xxh64 action="original">{}</xxh64>
    </hash>
  </hashes>
</hashlist>"#,
            hash
        )
    }

    #[test]
    fn parses_manifest_hashes() {
        let entries = parse_manifest_xml(sample_manifest_xml("abc").as_bytes()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "Clips/test.mov");
        assert_eq!(entries[0].hashes.get(&HashAlgorithm::XXH64).unwrap(), "abc");
    }

    #[test]
    fn verifies_directory_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let clips = root.join("Clips");
        fs::create_dir_all(&clips).unwrap();
        fs::write(clips.join("test.mov"), b"test video content").unwrap();

        let file_hash = hash_engine::hash_file_sync(
            &clips.join("test.mov"),
            &HashEngineConfig {
                algorithms: vec![HashAlgorithm::XXH64],
                buffer_size: 4 * 1024 * 1024,
            },
        )
        .unwrap()
        .remove(0)
        .hex_digest;

        let manifest_xml = sample_manifest_xml(&file_hash);
        let ascmhl_dir = root.join(ASCMHL_DIR_NAME);
        fs::create_dir_all(&ascmhl_dir).unwrap();
        let manifest_name = "0001_test_2024-01-01_000000Z.mhl";
        fs::write(ascmhl_dir.join(manifest_name), manifest_xml.as_bytes()).unwrap();

        let mut hasher = Sha256::new();
        hasher.update(manifest_xml.as_bytes());
        let manifest_hash = format!("{:x}", hasher.finalize());
        let chain_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<ascmhldirectory xmlns="urn:ASC:MHL:DIRECTORY:v2.0">
  <hashlist sequencenr="1">
    <path>{}</path>
    <sha256>{}</sha256>
  </hashlist>
</ascmhldirectory>"#,
            manifest_name, manifest_hash
        );
        fs::write(ascmhl_dir.join(CHAIN_FILE_NAME), chain_xml).unwrap();

        let report = verify_mhl_path(root, MhlVerifyOptions::default()).unwrap();
        assert!(report.summary.success);
        assert_eq!(report.summary.chain_valid, 1);
        assert_eq!(report.summary.passed, 1);
        assert!(report.issues.is_empty());
    }

    #[test]
    fn reports_hash_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("clip.mov"), b"changed").unwrap();

        let manifest = root.join("test.mhl");
        fs::write(
            &manifest,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<hashlist version="2.0" xmlns="urn:ASC:MHL:v2.0">
  <hashes>
    <hash>
      <path>clip.mov</path>
      <xxh64>0000000000000000</xxh64>
    </hash>
  </hashes>
</hashlist>"#,
        )
        .unwrap();

        let report = verify_mhl_path(&manifest, MhlVerifyOptions::default()).unwrap();
        assert!(!report.summary.success);
        assert_eq!(report.summary.failed, 1);
        assert_eq!(report.issues[0].kind, "hashMismatch");
    }

    #[test]
    fn chain_only_single_manifest_reports_parseable_files() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("test.mhl");
        fs::write(&manifest, sample_manifest_xml("abc")).unwrap();

        let report = verify_mhl_path(
            &manifest,
            MhlVerifyOptions {
                chain_only: true,
                ..Default::default()
            },
        )
        .unwrap();

        assert!(report.summary.success);
        assert_eq!(report.summary.total_files, 1);
        assert_eq!(report.summary.passed, 0);
    }

    #[test]
    fn rejects_manifest_paths_outside_root() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("test.mhl");
        fs::write(
            &manifest,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<hashlist version="2.0" xmlns="urn:ASC:MHL:v2.0">
  <hashes>
    <hash>
      <path>../outside.mov</path>
      <xxh64>0000000000000000</xxh64>
    </hash>
  </hashes>
</hashlist>"#,
        )
        .unwrap();

        let report = verify_mhl_path(&manifest, MhlVerifyOptions::default()).unwrap();

        assert!(!report.summary.success);
        assert_eq!(report.summary.errors, 1);
        assert_eq!(report.issues[0].kind, "unsafePath");
    }
}
