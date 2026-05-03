//! MHL Verify CLI — Standalone ASC MHL verification tool.
//!
//! A lightweight, cross-platform command-line tool for verifying
//! ASC MHL hash lists. Post-production houses can use this to verify
//! delivered media without installing the full DIT Pro.
//!
//! Usage:
//!   mhl-verify <path>                  Verify all MHL generations in a directory
//!   mhl-verify <path> --chain-only     Only verify chain integrity (no file hashing)
//!   mhl-verify <file.mhl>              Verify a single MHL manifest file

use anyhow::{bail, Context, Result};
use clap::Parser;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

// ─── CLI ─────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "mhl-verify",
    about = "Verify ASC MHL hash lists for media integrity",
    long_about = "A standalone tool to verify ASC MHL v2.0 hash lists.\n\
                   Checks chain integrity (manifest file SHA-256) and \
                   re-computes file hashes to detect corruption or tampering.\n\n\
                   Supports XXH64, XXH3, XXH128, SHA-256, and MD5.",
    version
)]
struct Args {
    /// Path to a directory containing an ascmhl/ folder, or a single .mhl file
    #[arg()]
    path: String,

    /// Only verify chain integrity (skip file hash re-computation)
    #[arg(long)]
    chain_only: bool,

    /// Verbose output (show each file result)
    #[arg(short, long)]
    verbose: bool,

    /// Quiet mode (only print errors and summary)
    #[arg(short, long)]
    quiet: bool,

    /// Verify a specific generation number only
    #[arg(long)]
    generation: Option<u32>,
}

// ─── Data Types ──────────────────────────────────────────────────────────────

/// Supported hash algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum HashAlgo {
    XXH64,
    XXH3,
    XXH128,
    SHA256,
    MD5,
}

impl HashAlgo {
    fn from_xml_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "xxh64" => Some(Self::XXH64),
            "xxh3" => Some(Self::XXH3),
            "xxh128" => Some(Self::XXH128),
            "sha256" | "sha-256" => Some(Self::SHA256),
            "md5" => Some(Self::MD5),
            _ => None,
        }
    }
}

impl fmt::Display for HashAlgo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HashAlgo::XXH64 => write!(f, "XXH64"),
            HashAlgo::XXH3 => write!(f, "XXH3"),
            HashAlgo::XXH128 => write!(f, "XXH128"),
            HashAlgo::SHA256 => write!(f, "SHA-256"),
            HashAlgo::MD5 => write!(f, "MD5"),
        }
    }
}

/// A hash entry parsed from an MHL manifest
#[derive(Debug, Clone)]
struct MhlFileEntry {
    /// Relative path from media root
    path: String,
    /// File size (from manifest, for informational display)
    #[allow(dead_code)]
    file_size: u64,
    /// Hash values keyed by algorithm
    hashes: HashMap<HashAlgo, String>,
}

/// A chain entry from ascmhl_chain.xml
#[derive(Debug, Clone)]
struct ChainEntry {
    sequence_nr: u32,
    path: String,
    reference_hash: String,
}

/// Result of verifying a single file
#[derive(Debug)]
enum FileVerifyResult {
    /// All hashes match
    Pass,
    /// Hash mismatch detected
    Mismatch {
        algorithm: HashAlgo,
        expected: String,
        actual: String,
    },
    /// File not found on disk
    Missing,
    /// IO error reading file
    Error(String),
}

/// Summary of verification results
#[derive(Debug, Default)]
struct VerifySummary {
    total_files: usize,
    passed: usize,
    failed: usize,
    missing: usize,
    errors: usize,
    chain_entries: usize,
    chain_valid: usize,
    chain_invalid: usize,
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let args = Args::parse();

    if !args.quiet {
        println!("mhl-verify v{}", env!("CARGO_PKG_VERSION"));
        println!();
    }

    let path = PathBuf::from(&args.path);

    if !path.exists() {
        bail!("Path does not exist: {}", args.path);
    }

    let start = Instant::now();

    // Determine if we're verifying a single file or a directory
    if path.is_file() && path.extension().is_some_and(|e| e == "mhl") {
        // Single MHL file verification
        verify_single_manifest(&path, &args)?;
    } else if path.is_dir() {
        // Directory verification (look for ascmhl/)
        verify_directory(&path, &args)?;
    } else {
        bail!(
            "Path must be a directory containing ascmhl/ or a .mhl file: {}",
            args.path
        );
    }

    let elapsed = start.elapsed();
    if !args.quiet {
        println!();
        println!("Completed in {:.1}s", elapsed.as_secs_f64());
    }

    Ok(())
}

// ─── Directory Verification ──────────────────────────────────────────────────

fn verify_directory(root: &Path, args: &Args) -> Result<()> {
    let ascmhl_dir = root.join("ascmhl");
    if !ascmhl_dir.exists() {
        bail!("No ascmhl/ directory found in: {}", root.display());
    }

    let chain_path = ascmhl_dir.join("ascmhl_chain.xml");
    if !chain_path.exists() {
        bail!("No ascmhl_chain.xml found in: {}", ascmhl_dir.display());
    }

    if !args.quiet {
        println!("Verifying: {}", root.display());
        println!();
    }

    // Parse chain file
    let chain = parse_chain_file(&chain_path)?;

    if chain.is_empty() {
        bail!("Chain file is empty — no generations found");
    }

    if !args.quiet {
        println!("Found {} generation(s) in chain", chain.len());
        println!();
    }

    let mut summary = VerifySummary {
        chain_entries: chain.len(),
        ..Default::default()
    };

    // Step 1: Verify chain integrity (SHA-256 of each manifest file)
    if !args.quiet {
        println!("--- Chain Integrity ---");
    }

    for entry in &chain {
        let manifest_path = ascmhl_dir.join(&entry.path);
        let result = verify_chain_entry(&manifest_path, &entry.reference_hash);

        match result {
            Ok(true) => {
                summary.chain_valid += 1;
                if !args.quiet {
                    println!("  [PASS] Gen {:04}: {}", entry.sequence_nr, entry.path);
                }
            }
            Ok(false) => {
                summary.chain_invalid += 1;
                println!(
                    "  [FAIL] Gen {:04}: {} (SHA-256 mismatch — file may be tampered!)",
                    entry.sequence_nr, entry.path
                );
            }
            Err(e) => {
                summary.chain_invalid += 1;
                println!(
                    "  [ERR]  Gen {:04}: {} ({})",
                    entry.sequence_nr, entry.path, e
                );
            }
        }
    }

    if !args.quiet {
        println!();
    }

    // Step 2: Verify file hashes (unless chain-only mode)
    if !args.chain_only {
        // Decide which generations to verify
        let gens_to_verify: Vec<&ChainEntry> = if let Some(gen_nr) = args.generation {
            chain.iter().filter(|e| e.sequence_nr == gen_nr).collect()
        } else {
            // By default, verify the latest generation
            chain.last().into_iter().collect()
        };

        for chain_entry in &gens_to_verify {
            let manifest_path = ascmhl_dir.join(&chain_entry.path);
            if !args.quiet {
                println!(
                    "--- File Verification (Gen {:04}) ---",
                    chain_entry.sequence_nr
                );
            }

            let entries = parse_manifest(&manifest_path)?;
            verify_file_hashes(root, &entries, args, &mut summary)?;
        }
    }

    // Print summary
    print_summary(&summary, args.chain_only);

    // Exit with non-zero if any failures
    if summary.chain_invalid > 0 || summary.failed > 0 || summary.missing > 0 {
        std::process::exit(1);
    }

    Ok(())
}

// ─── Single Manifest Verification ────────────────────────────────────────────

fn verify_single_manifest(manifest_path: &Path, args: &Args) -> Result<()> {
    if !args.quiet {
        println!("Verifying manifest: {}", manifest_path.display());
        println!();
    }

    // Determine root directory (parent of ascmhl/)
    let root = manifest_path
        .parent()
        .and_then(|p| {
            if p.file_name().is_some_and(|n| n == "ascmhl") {
                p.parent()
            } else {
                Some(p)
            }
        })
        .unwrap_or_else(|| Path::new("."));

    let entries = parse_manifest(manifest_path)?;

    if !args.quiet {
        println!("Found {} file entries", entries.len());
        println!();
    }

    let mut summary = VerifySummary::default();

    if !args.chain_only {
        verify_file_hashes(root, &entries, args, &mut summary)?;
    }

    print_summary(&summary, args.chain_only);

    if summary.failed > 0 || summary.missing > 0 {
        std::process::exit(1);
    }

    Ok(())
}

// ─── File Hash Verification ──────────────────────────────────────────────────

fn verify_file_hashes(
    root: &Path,
    entries: &[MhlFileEntry],
    args: &Args,
    summary: &mut VerifySummary,
) -> Result<()> {
    summary.total_files += entries.len();

    for entry in entries {
        let file_path = root.join(&entry.path);

        let result = if !file_path.exists() {
            FileVerifyResult::Missing
        } else {
            verify_single_file(&file_path, entry)
        };

        match &result {
            FileVerifyResult::Pass => {
                summary.passed += 1;
                if args.verbose && !args.quiet {
                    println!("  [PASS] {}", entry.path);
                }
            }
            FileVerifyResult::Mismatch {
                algorithm,
                expected,
                actual,
            } => {
                summary.failed += 1;
                println!(
                    "  [FAIL] {} ({}: expected {}, got {})",
                    entry.path, algorithm, expected, actual
                );
            }
            FileVerifyResult::Missing => {
                summary.missing += 1;
                println!("  [MISS] {} (file not found)", entry.path);
            }
            FileVerifyResult::Error(msg) => {
                summary.errors += 1;
                println!("  [ERR]  {} ({})", entry.path, msg);
            }
        }
    }

    Ok(())
}

fn verify_single_file(file_path: &Path, entry: &MhlFileEntry) -> FileVerifyResult {
    // Compute hashes for all algorithms present in the manifest entry
    for (algo, expected_hash) in &entry.hashes {
        match compute_file_hash(file_path, *algo) {
            Ok(actual_hash) => {
                if actual_hash.to_lowercase() != expected_hash.to_lowercase() {
                    return FileVerifyResult::Mismatch {
                        algorithm: *algo,
                        expected: expected_hash.clone(),
                        actual: actual_hash,
                    };
                }
            }
            Err(e) => {
                return FileVerifyResult::Error(e.to_string());
            }
        }
    }

    FileVerifyResult::Pass
}

// ─── Hash Computation ────────────────────────────────────────────────────────

const BUFFER_SIZE: usize = 4 * 1024 * 1024; // 4 MB

fn compute_file_hash(path: &Path, algo: HashAlgo) -> Result<String> {
    let mut file =
        fs::File::open(path).with_context(|| format!("Failed to open: {}", path.display()))?;

    let mut buf = vec![0u8; BUFFER_SIZE];

    match algo {
        HashAlgo::XXH64 => {
            let mut hasher = xxhash_rust::xxh64::Xxh64::new(0);
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(format!("{:016x}", hasher.digest()))
        }
        HashAlgo::XXH3 => {
            let mut hasher = xxhash_rust::xxh3::Xxh3::new();
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(format!("{:016x}", hasher.digest()))
        }
        HashAlgo::XXH128 => {
            let mut hasher = xxhash_rust::xxh3::Xxh3::new();
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(format!("{:032x}", hasher.digest128()))
        }
        HashAlgo::SHA256 => {
            use sha2::Digest;
            let mut hasher = sha2::Sha256::new();
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(format!("{:x}", hasher.finalize()))
        }
        HashAlgo::MD5 => {
            use md5::Digest;
            let mut hasher = md5::Md5::new();
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(format!("{:x}", hasher.finalize()))
        }
    }
}

// ─── Chain Verification ──────────────────────────────────────────────────────

fn verify_chain_entry(manifest_path: &Path, expected_hash: &str) -> Result<bool> {
    use sha2::Digest;

    if !manifest_path.exists() {
        bail!("Manifest file not found: {}", manifest_path.display());
    }

    let bytes = fs::read(manifest_path)
        .with_context(|| format!("Failed to read: {}", manifest_path.display()))?;

    let mut hasher = sha2::Sha256::new();
    hasher.update(&bytes);
    let computed = format!("{:x}", hasher.finalize());

    Ok(computed == expected_hash)
}

// ─── XML Parsing ─────────────────────────────────────────────────────────────

/// Parse ascmhl_chain.xml
fn parse_chain_file(path: &Path) -> Result<Vec<ChainEntry>> {
    let content =
        fs::read(path).with_context(|| format!("Failed to read chain file: {}", path.display()))?;

    parse_chain_xml(&content)
}

fn parse_chain_xml(xml_bytes: &[u8]) -> Result<Vec<ChainEntry>> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_reader(xml_bytes);
    reader.trim_text(true);

    let mut entries: Vec<ChainEntry> = Vec::new();
    let mut current_entry: Option<ChainEntry> = None;
    let mut current_element = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                match name.as_str() {
                    "hashlist" => {
                        let mut seq_nr: u32 = 0;
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"sequencenr" {
                                seq_nr = String::from_utf8_lossy(&attr.value).parse().unwrap_or(0);
                            }
                        }
                        current_entry = Some(ChainEntry {
                            sequence_nr: seq_nr,
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

/// Parse an ASC MHL manifest file and extract file hash entries.
fn parse_manifest(path: &Path) -> Result<Vec<MhlFileEntry>> {
    let content =
        fs::read(path).with_context(|| format!("Failed to read manifest: {}", path.display()))?;

    parse_manifest_xml(&content)
}

fn parse_manifest_xml(xml_bytes: &[u8]) -> Result<Vec<MhlFileEntry>> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_reader(xml_bytes);
    reader.trim_text(true);

    let mut entries: Vec<MhlFileEntry> = Vec::new();
    let mut buf = Vec::new();

    // State machine for parsing
    let mut in_hash_block = false; // inside a <hash> element (file entry)
    let mut in_hashes_section = false; // inside <hashes> section
    let mut current_file_path = String::new();
    let mut current_file_size: u64 = 0;
    let mut current_hashes: HashMap<HashAlgo, String> = HashMap::new();
    let mut current_element = String::new();
    let mut current_algo: Option<HashAlgo> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();

                match name.as_str() {
                    "hashes" => {
                        in_hashes_section = true;
                    }
                    "hash" if in_hashes_section => {
                        in_hash_block = true;
                        current_file_path.clear();
                        current_file_size = 0;
                        current_hashes.clear();
                    }
                    "path" if in_hash_block => {
                        current_element = "path".to_string();
                        // Extract size attribute
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"size" {
                                current_file_size =
                                    String::from_utf8_lossy(&attr.value).parse().unwrap_or(0);
                            }
                        }
                    }
                    algo_name if in_hash_block => {
                        if let Some(algo) = HashAlgo::from_xml_name(algo_name) {
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
                            entries.push(MhlFileEntry {
                                path: current_file_path.clone(),
                                file_size: current_file_size,
                                hashes: current_hashes.clone(),
                            });
                        }
                        in_hash_block = false;
                    }
                    "hashes" => {
                        in_hashes_section = false;
                    }
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

// ─── Summary ─────────────────────────────────────────────────────────────────

fn print_summary(summary: &VerifySummary, chain_only: bool) {
    println!();
    println!("=== Verification Summary ===");

    // Chain results
    if summary.chain_entries > 0 {
        println!(
            "Chain:  {}/{} generations verified",
            summary.chain_valid, summary.chain_entries
        );
        if summary.chain_invalid > 0 {
            println!(
                "        {} TAMPERED (chain integrity compromised!)",
                summary.chain_invalid
            );
        }
    }

    // File results
    if !chain_only && summary.total_files > 0 {
        println!(
            "Files:  {}/{} verified OK",
            summary.passed, summary.total_files
        );
        if summary.failed > 0 {
            println!(
                "        {} FAILED (hash mismatch — data may be corrupted!)",
                summary.failed
            );
        }
        if summary.missing > 0 {
            println!(
                "        {} MISSING (files not found on disk)",
                summary.missing
            );
        }
        if summary.errors > 0 {
            println!("        {} errors (could not read files)", summary.errors);
        }
    }

    // Overall verdict
    let all_ok = summary.chain_invalid == 0
        && summary.failed == 0
        && summary.missing == 0
        && summary.errors == 0;

    println!();
    if all_ok {
        println!("Result: PASS — All checks passed.");
    } else {
        println!("Result: FAIL — Issues detected. See details above.");
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn sample_chain_xml() -> &'static str {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ascmhldirectory xmlns="urn:ASC:MHL:DIRECTORY:v2.0">
  <hashlist sequencenr="1">
    <path>0001_Media_2024-01-15_120000Z.mhl</path>
    <sha256>abc123def456</sha256>
  </hashlist>
  <hashlist sequencenr="2">
    <path>0002_Media_2024-01-16_090000Z.mhl</path>
    <sha256>789xyz000111</sha256>
  </hashlist>
</ascmhldirectory>"#
    }

    fn sample_manifest_xml() -> &'static str {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<hashlist version="2.0" xmlns="urn:ASC:MHL:v2.0">
  <creatorinfo>
    <creationdate>2024-06-15T10:30:00+00:00</creationdate>
    <tool version="0.1.0">DIT Pro</tool>
  </creatorinfo>
  <processinfo>
    <process>transfer</process>
  </processinfo>
  <hashes>
    <hash>
      <path size="1073741824" lastmodificationdate="2024-06-15T10:30:00+00:00">Clips/A002C006.mov</path>
      <xxh64 action="original" hashdate="2024-06-15T10:30:00+00:00">0ea03b369a463d9d</xxh64>
    </hash>
    <hash>
      <path size="536870912" lastmodificationdate="2024-06-15T10:30:00+00:00">Clips/A002C007.mov</path>
      <xxh64 action="original" hashdate="2024-06-15T10:30:00+00:00">7680e5f98f4a80fd</xxh64>
      <sha256 action="original" hashdate="2024-06-15T10:30:00+00:00">a1b2c3d4e5f6</sha256>
    </hash>
  </hashes>
</hashlist>"#
    }

    #[test]
    fn test_parse_chain_xml() {
        let entries = parse_chain_xml(sample_chain_xml().as_bytes()).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].sequence_nr, 1);
        assert_eq!(entries[0].path, "0001_Media_2024-01-15_120000Z.mhl");
        assert_eq!(entries[0].reference_hash, "abc123def456");
        assert_eq!(entries[1].sequence_nr, 2);
    }

    #[test]
    fn test_parse_manifest_xml() {
        let entries = parse_manifest_xml(sample_manifest_xml().as_bytes()).unwrap();
        assert_eq!(entries.len(), 2);

        // First entry: single hash
        assert_eq!(entries[0].path, "Clips/A002C006.mov");
        assert_eq!(entries[0].file_size, 1073741824);
        assert_eq!(entries[0].hashes.len(), 1);
        assert_eq!(
            entries[0].hashes.get(&HashAlgo::XXH64).unwrap(),
            "0ea03b369a463d9d"
        );

        // Second entry: two hashes
        assert_eq!(entries[1].path, "Clips/A002C007.mov");
        assert_eq!(entries[1].hashes.len(), 2);
        assert!(entries[1].hashes.contains_key(&HashAlgo::XXH64));
        assert!(entries[1].hashes.contains_key(&HashAlgo::SHA256));
    }

    #[test]
    fn test_parse_empty_chain() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<ascmhldirectory xmlns="urn:ASC:MHL:DIRECTORY:v2.0">
</ascmhldirectory>"#;
        let entries = parse_chain_xml(xml.as_bytes()).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_hash_algo_from_xml_name() {
        assert_eq!(HashAlgo::from_xml_name("xxh64"), Some(HashAlgo::XXH64));
        assert_eq!(HashAlgo::from_xml_name("xxh3"), Some(HashAlgo::XXH3));
        assert_eq!(HashAlgo::from_xml_name("xxh128"), Some(HashAlgo::XXH128));
        assert_eq!(HashAlgo::from_xml_name("sha256"), Some(HashAlgo::SHA256));
        assert_eq!(HashAlgo::from_xml_name("md5"), Some(HashAlgo::MD5));
        assert_eq!(HashAlgo::from_xml_name("unknown"), None);
    }

    #[test]
    fn test_compute_xxh64_hash() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.bin");
        let mut f = fs::File::create(&file_path).unwrap();
        f.write_all(b"Hello, World!").unwrap();

        let hash = compute_file_hash(&file_path, HashAlgo::XXH64).unwrap();
        // XXH64 of "Hello, World!" with seed 0
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 16); // 64-bit = 16 hex chars
    }

    #[test]
    fn test_compute_sha256_hash() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.bin");
        let mut f = fs::File::create(&file_path).unwrap();
        f.write_all(b"Hello, World!").unwrap();

        let hash = compute_file_hash(&file_path, HashAlgo::SHA256).unwrap();
        // SHA-256 of "Hello, World!"
        assert_eq!(
            hash,
            "dffd6021bb2bd5b0af676290809ec3a53191dd81c7f70a4b28688a362182986f"
        );
    }

    #[test]
    fn test_compute_md5_hash() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.bin");
        let mut f = fs::File::create(&file_path).unwrap();
        f.write_all(b"Hello, World!").unwrap();

        let hash = compute_file_hash(&file_path, HashAlgo::MD5).unwrap();
        assert_eq!(hash, "65a8e27d8879283831b664bd8b7f0ad4");
    }

    #[test]
    fn test_verify_chain_entry_valid() {
        use sha2::Digest;

        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("test.mhl");
        let content = b"<hashlist>test content</hashlist>";
        fs::write(&manifest_path, content).unwrap();

        let mut hasher = sha2::Sha256::new();
        hasher.update(content);
        let expected_hash = format!("{:x}", hasher.finalize());

        assert!(verify_chain_entry(&manifest_path, &expected_hash).unwrap());
    }

    #[test]
    fn test_verify_chain_entry_tampered() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("test.mhl");
        fs::write(&manifest_path, b"modified content").unwrap();

        assert!(!verify_chain_entry(&manifest_path, "wrong_hash_value").unwrap());
    }

    #[test]
    fn test_verify_single_file_pass() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("clip.mov");
        fs::write(&file_path, b"video data").unwrap();

        // Compute actual hash
        let actual_hash = compute_file_hash(&file_path, HashAlgo::XXH64).unwrap();

        let entry = MhlFileEntry {
            path: "clip.mov".to_string(),
            file_size: 10,
            hashes: HashMap::from([(HashAlgo::XXH64, actual_hash)]),
        };

        match verify_single_file(&file_path, &entry) {
            FileVerifyResult::Pass => {} // expected
            other => panic!("Expected Pass, got {:?}", other),
        }
    }

    #[test]
    fn test_verify_single_file_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("clip.mov");
        fs::write(&file_path, b"video data").unwrap();

        let entry = MhlFileEntry {
            path: "clip.mov".to_string(),
            file_size: 10,
            hashes: HashMap::from([(HashAlgo::XXH64, "0000000000000000".to_string())]),
        };

        match verify_single_file(&file_path, &entry) {
            FileVerifyResult::Mismatch { algorithm, .. } => {
                assert_eq!(algorithm, HashAlgo::XXH64);
            }
            other => panic!("Expected Mismatch, got {:?}", other),
        }
    }

    #[test]
    fn test_end_to_end_directory_verification() {
        use sha2::Digest;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // Create test file
        let clips_dir = root.join("Clips");
        fs::create_dir_all(&clips_dir).unwrap();
        fs::write(clips_dir.join("test.mov"), b"test video content").unwrap();

        // Compute file hash
        let file_hash = compute_file_hash(&clips_dir.join("test.mov"), HashAlgo::XXH64).unwrap();

        // Create manifest XML
        let manifest_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<hashlist version="2.0" xmlns="urn:ASC:MHL:v2.0">
  <creatorinfo>
    <creationdate>2024-01-01T00:00:00+00:00</creationdate>
    <tool version="0.1.0">DIT Pro</tool>
  </creatorinfo>
  <processinfo>
    <process>transfer</process>
  </processinfo>
  <hashes>
    <hash>
      <path size="18">Clips/test.mov</path>
      <xxh64 action="original">{}</xxh64>
    </hash>
  </hashes>
</hashlist>"#,
            file_hash
        );

        // Create ascmhl directory
        let ascmhl_dir = root.join("ascmhl");
        fs::create_dir_all(&ascmhl_dir).unwrap();

        let manifest_filename = "0001_test_2024-01-01_000000Z.mhl";
        let manifest_path = ascmhl_dir.join(manifest_filename);
        fs::write(&manifest_path, manifest_xml.as_bytes()).unwrap();

        // Compute chain hash
        let mut hasher = sha2::Sha256::new();
        hasher.update(manifest_xml.as_bytes());
        let chain_hash = format!("{:x}", hasher.finalize());

        // Create chain file
        let chain_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<ascmhldirectory xmlns="urn:ASC:MHL:DIRECTORY:v2.0">
  <hashlist sequencenr="1">
    <path>{}</path>
    <sha256>{}</sha256>
  </hashlist>
</ascmhldirectory>"#,
            manifest_filename, chain_hash
        );
        fs::write(ascmhl_dir.join("ascmhl_chain.xml"), chain_xml).unwrap();

        // Now verify
        let args = Args {
            path: root.to_string_lossy().to_string(),
            chain_only: false,
            verbose: true,
            quiet: false,
            generation: None,
        };

        // This should succeed without error
        verify_directory(root, &args).unwrap();
    }
}
