//! AC Verification Tests — MVP Acceptance Criteria (v1.0.0-beta.1)
//!
//! Tests each AC (Acceptance Criterion) from Phase 1 spec:
//!   AC-3: MHL format compliance (ASC MHL v2.0 standard)
//!   AC-5: Report content completeness (file list + hash info)
//!   AC-7: MHL app-integrated verification
//!
//! Note: AC-1/2/4/6 are covered by stress_tests.rs
//!
//! Run with:
//!   cargo test --test ac_verification -- --nocapture

use app_lib::checkpoint;
use app_lib::db;
use app_lib::hash_engine::{HashAlgorithm, HashEngineConfig, HashResult};
use app_lib::mhl::{
    self,
    verifier::{verify_mhl_path, MhlVerifyOptions},
    MhlConfig, MhlProcessType, ASCMHL_DIR_NAME, CHAIN_FILE_NAME, CHAIN_NAMESPACE, MHL_NAMESPACE,
};
use app_lib::report;
use app_lib::workflow::{OffloadConfig, OffloadEvent, OffloadWorkflow};
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

// ─── Helpers ────────────────────────────────────────────────────────────────

fn setup_db(db_path: &Path) -> Connection {
    db::init_database(db_path.to_str().unwrap()).unwrap()
}

fn wrap_db(conn: Connection) -> Arc<std::sync::Mutex<Connection>> {
    Arc::new(std::sync::Mutex::new(conn))
}

/// Create test files with known content in a directory.
fn create_test_files(dir: &Path, count: usize, size: usize) -> Vec<String> {
    std::fs::create_dir_all(dir).unwrap();
    let mut names = Vec::new();
    for i in 0..count {
        let name = format!("A001C{:04}.mov", i + 1);
        let path = dir.join(&name);
        let data: Vec<u8> = (0..size).map(|b| ((b + i * 7) % 256) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        names.push(name);
    }
    names
}

// ═══════════════════════════════════════════════════════════════════════════
// AC-3: ASC MHL Format Compliance
// ═══════════════════════════════════════════════════════════════════════════

/// AC-3.1: Manifest XML uses correct ASC MHL v2.0 namespace and structure
#[tokio::test]
async fn ac3_manifest_xml_namespace_and_structure() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("MediaCard");
    create_test_files(&root, 3, 1024);

    let mut history = mhl::load_or_create_history(&root).await.unwrap();

    // Build hash entries as if from copy engine
    let mut file_hashes = HashMap::new();
    let mut file_metadata = HashMap::new();
    let now = chrono::Utc::now();

    for i in 1..=3 {
        let name = format!("A001C{:04}.mov", i);
        let path = root.join(&name);
        let data = std::fs::read(&path).unwrap();

        // Compute real XXH64 hash
        use xxhash_rust::xxh64::Xxh64;
        let mut hasher = Xxh64::new(0);
        hasher.update(&data);
        let hex = format!("{:016x}", hasher.digest());

        file_hashes.insert(
            name.clone(),
            vec![HashResult {
                algorithm: HashAlgorithm::XXH64,
                hex_digest: hex,
            }],
        );
        file_metadata.insert(name, (data.len() as u64, now));
    }

    let config = MhlConfig::default();
    let manifest_path = mhl::create_generation(
        &mut history,
        &file_hashes,
        &file_metadata,
        MhlProcessType::Transfer,
        &config,
    )
    .await
    .unwrap();

    // Read and validate the manifest XML
    let xml_content = std::fs::read_to_string(&manifest_path).unwrap();

    // Required: XML declaration
    assert!(
        xml_content.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"),
        "Manifest must start with XML declaration"
    );

    // Required: ASC MHL v2.0 namespace
    assert!(
        xml_content.contains(&format!("xmlns=\"{}\"", MHL_NAMESPACE)),
        "Manifest must use ASC MHL v2.0 namespace: {}",
        MHL_NAMESPACE
    );

    // Required: version="2.0"
    assert!(
        xml_content.contains("version=\"2.0\""),
        "Manifest must declare version 2.0"
    );

    // Required sections
    assert!(
        xml_content.contains("<creatorinfo>"),
        "Manifest must contain <creatorinfo>"
    );
    assert!(
        xml_content.contains("<processinfo>"),
        "Manifest must contain <processinfo>"
    );
    assert!(
        xml_content.contains("<hashes>"),
        "Manifest must contain <hashes>"
    );

    // Required creatorinfo elements
    assert!(
        xml_content.contains("<creationdate>"),
        "Manifest must contain <creationdate>"
    );
    assert!(
        xml_content.contains("<tool"),
        "Manifest must contain <tool> with version"
    );

    // Required processinfo elements
    assert!(
        xml_content.contains("<process>transfer</process>"),
        "Manifest must contain process type"
    );
    assert!(
        xml_content.contains("<roothash>"),
        "Manifest must contain root hash"
    );
    assert!(
        xml_content.contains("<ignore>"),
        "Manifest must contain ignore patterns"
    );

    println!("[AC-3.1] PASS: Manifest XML has correct ASC MHL v2.0 namespace and structure");
}

/// AC-3.2: Hash entries contain required attributes (path, size, algorithm, action, hashdate)
#[tokio::test]
async fn ac3_hash_entries_required_attributes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("TestMedia");
    create_test_files(&root, 2, 2048);

    let mut history = mhl::load_or_create_history(&root).await.unwrap();
    let mut file_hashes = HashMap::new();
    let mut file_metadata = HashMap::new();
    let now = chrono::Utc::now();

    for i in 1..=2 {
        let name = format!("A001C{:04}.mov", i);
        file_hashes.insert(
            name.clone(),
            vec![HashResult {
                algorithm: HashAlgorithm::XXH64,
                hex_digest: format!("{:016x}", i * 12345),
            }],
        );
        file_metadata.insert(name, (2048u64, now));
    }

    let config = MhlConfig::default();
    let manifest_path = mhl::create_generation(
        &mut history,
        &file_hashes,
        &file_metadata,
        MhlProcessType::Transfer,
        &config,
    )
    .await
    .unwrap();

    let xml = std::fs::read_to_string(&manifest_path).unwrap();

    // Verify hash entries have required attributes
    // Each <hash> block should contain:
    //   <path size="..." lastmodificationdate="...">filename</path>
    //   <xxh64 action="..." hashdate="...">hexvalue</xxh64>
    assert!(
        xml.contains("size=\"2048\""),
        "Hash entry must include file size"
    );
    assert!(
        xml.contains("lastmodificationdate="),
        "Hash entry must include last modification date"
    );
    assert!(
        xml.contains("action=\"original\""),
        "First generation entries must have action=original"
    );
    assert!(
        xml.contains("hashdate="),
        "Hash entry must include hash date"
    );
    assert!(
        xml.contains("<xxh64"),
        "Hash entry must use correct algorithm element name"
    );

    // Verify file paths are present
    assert!(
        xml.contains("A001C0001.mov"),
        "Hash entry must include file path"
    );
    assert!(
        xml.contains("A001C0002.mov"),
        "Hash entry must include file path"
    );

    println!("[AC-3.2] PASS: Hash entries contain all required attributes");
}

/// AC-3.3: Chain file uses correct namespace and tracks manifest SHA-256
#[tokio::test]
async fn ac3_chain_file_integrity() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ChainTestMedia");
    create_test_files(&root, 5, 512);

    let mut history = mhl::load_or_create_history(&root).await.unwrap();
    let now = chrono::Utc::now();

    // Create 2 generations
    for gen in 0..2 {
        let mut file_hashes = HashMap::new();
        let mut file_metadata = HashMap::new();
        for i in 1..=5 {
            let name = format!("A001C{:04}.mov", i);
            file_hashes.insert(
                name.clone(),
                vec![HashResult {
                    algorithm: HashAlgorithm::XXH64,
                    hex_digest: format!("{:016x}", i * 100 + gen),
                }],
            );
            file_metadata.insert(name, (512u64, now));
        }

        let process = if gen == 0 {
            MhlProcessType::Transfer
        } else {
            MhlProcessType::InPlace
        };

        mhl::create_generation(
            &mut history,
            &file_hashes,
            &file_metadata,
            process,
            &MhlConfig::default(),
        )
        .await
        .unwrap();
    }

    // Read chain file
    let chain_path = root.join(ASCMHL_DIR_NAME).join(CHAIN_FILE_NAME);
    assert!(chain_path.exists(), "Chain file must exist");

    let chain_xml = std::fs::read_to_string(&chain_path).unwrap();

    // Verify chain namespace
    assert!(
        chain_xml.contains(&format!("xmlns=\"{}\"", CHAIN_NAMESPACE)),
        "Chain file must use ASC MHL DIRECTORY v2.0 namespace"
    );

    // Verify 2 generations tracked
    assert!(
        chain_xml.contains("sequencenr=\"1\""),
        "Chain must contain generation 1"
    );
    assert!(
        chain_xml.contains("sequencenr=\"2\""),
        "Chain must contain generation 2"
    );

    // Verify chain integrity (SHA-256 of each manifest matches chain)
    let results = mhl::verify_chain(&history).await.unwrap();
    assert_eq!(results.len(), 2, "Chain must have 2 entries");
    for (seq, valid) in &results {
        assert!(
            *valid,
            "Chain entry {} must be valid (manifest SHA-256 matches)",
            seq
        );
    }

    println!("[AC-3.3] PASS: Chain file has correct namespace and SHA-256 integrity");
}

/// AC-3.4: Multi-algorithm support in MHL entries
#[tokio::test]
async fn ac3_multi_algorithm_entries() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("MultiAlgoMedia");
    create_test_files(&root, 1, 1024);

    let mut history = mhl::load_or_create_history(&root).await.unwrap();
    let now = chrono::Utc::now();

    let mut file_hashes = HashMap::new();
    let mut file_metadata = HashMap::new();

    // Provide both XXH64 and SHA256 hashes
    file_hashes.insert(
        "A001C0001.mov".to_string(),
        vec![
            HashResult {
                algorithm: HashAlgorithm::XXH64,
                hex_digest: "abcdef0123456789".to_string(),
            },
            HashResult {
                algorithm: HashAlgorithm::SHA256,
                hex_digest: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                    .to_string(),
            },
        ],
    );
    file_metadata.insert("A001C0001.mov".to_string(), (1024u64, now));

    let manifest_path = mhl::create_generation(
        &mut history,
        &file_hashes,
        &file_metadata,
        MhlProcessType::Transfer,
        &MhlConfig::default(),
    )
    .await
    .unwrap();

    let xml = std::fs::read_to_string(&manifest_path).unwrap();

    // Both algorithm elements must be present in the same <hash> block
    assert!(xml.contains("<xxh64"), "Must contain XXH64 hash element");
    assert!(xml.contains("<sha256"), "Must contain SHA256 hash element");
    assert!(
        xml.contains("abcdef0123456789"),
        "Must contain XXH64 hex value"
    );
    assert!(
        xml.contains("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
        "Must contain SHA256 hex value"
    );

    println!("[AC-3.4] PASS: Multi-algorithm entries correctly encoded in MHL");
}

/// AC-3.5: Full offload pipeline generates verifiable MHL
#[tokio::test]
async fn ac3_offload_pipeline_generates_verifiable_mhl() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source_card");
    let dest = dir.path().join("backup_a");
    let db_path = dir.path().join("test.db");

    create_test_files(&source, 10, 4096);
    std::fs::create_dir_all(&dest).unwrap();

    let conn = setup_db(&db_path);
    let db = wrap_db(conn);

    let config = OffloadConfig {
        source_path: source.clone(),
        dest_paths: vec![dest.clone()],
        hash_algorithms: vec![HashAlgorithm::XXH64, HashAlgorithm::SHA256],
        source_verify: true,
        post_verify: true,
        generate_mhl: true,
        cascade: false,
        ..Default::default()
    };

    let (tx, _rx) = mpsc::unbounded_channel::<OffloadEvent>();
    let wf = OffloadWorkflow::new(config, db.clone(), tx);
    let result = wf.execute().await;
    assert!(result.is_ok(), "Offload must succeed");

    let outcome = result.unwrap();
    assert!(outcome.success, "Offload outcome must be success");
    assert_eq!(outcome.failed_files, 0, "No files should fail");

    // Verify MHL was generated in dest
    let dest_ascmhl = dest.join(ASCMHL_DIR_NAME);
    assert!(dest_ascmhl.exists(), "ascmhl directory must exist in dest");

    let chain_path = dest_ascmhl.join(CHAIN_FILE_NAME);
    assert!(chain_path.exists(), "ascmhl_chain.xml must exist");

    // Load history and verify chain integrity
    let history = mhl::load_or_create_history(&dest).await.unwrap();
    assert!(
        !history.chain.is_empty(),
        "Chain must have at least one generation"
    );

    let chain_results = mhl::verify_chain(&history).await.unwrap();
    for (seq, valid) in &chain_results {
        assert!(
            *valid,
            "Chain entry {} must be valid after full offload",
            seq
        );
    }

    // Read manifest and verify all 10 files are tracked
    let manifest_path = dest_ascmhl.join(&history.chain[0].path);
    let xml = std::fs::read_to_string(&manifest_path).unwrap();

    for i in 1..=10 {
        let filename = format!("A001C{:04}.mov", i);
        assert!(
            xml.contains(&filename),
            "Manifest must contain file: {}",
            filename
        );
    }

    // Verify both hash algorithms are present
    assert!(xml.contains("<xxh64"), "Manifest must contain XXH64 hashes");
    assert!(
        xml.contains("<sha256"),
        "Manifest must contain SHA256 hashes"
    );

    println!("[AC-3.5] PASS: Full offload pipeline generates verifiable MHL with all files");
}

// ═══════════════════════════════════════════════════════════════════════════
// AC-5: Report Content Completeness
// ═══════════════════════════════════════════════════════════════════════════

/// AC-5.1: Day report HTML contains complete statistics
#[test]
fn ac5_day_report_html_complete_statistics() {
    let report_data = report::DayReport {
        date: "2026-02-25".to_string(),
        jobs: vec![
            report::JobSummary {
                job_id: "job-001".to_string(),
                job_name: "Card A Offload".to_string(),
                status: "completed".to_string(),
                source_path: "/Volumes/LUMIX/DCIM".to_string(),
                total_files: 50,
                completed_files: 50,
                failed_files: 0,
                total_bytes: 25_000_000_000,
                completed_bytes: 25_000_000_000,
                created_at: "2026-02-25T10:00:00".to_string(),
            },
            report::JobSummary {
                job_id: "job-002".to_string(),
                job_name: "Card B Offload".to_string(),
                status: "completed".to_string(),
                source_path: "/Volumes/RED/R3D".to_string(),
                total_files: 30,
                completed_files: 30,
                failed_files: 0,
                total_bytes: 100_000_000_000,
                completed_bytes: 100_000_000_000,
                created_at: "2026-02-25T14:00:00".to_string(),
            },
        ],
        total_files: 80,
        total_bytes: 125_000_000_000,
        total_jobs: 2,
        completed_jobs: 2,
        failed_jobs: 0,
    };

    let html = report::render_day_report_html(&report_data);

    // Must be valid HTML
    assert!(html.contains("<!DOCTYPE html>"), "Must be valid HTML5");
    assert!(html.contains("<html"), "Must contain HTML tag");
    assert!(html.contains("</html>"), "Must close HTML tag");

    // Must contain the date
    assert!(
        html.contains("2026-02-25"),
        "Report must contain the report date"
    );

    // Must contain job count
    assert!(
        html.contains(">2<"),
        "Report must display total job count (2)"
    );

    // Must contain file count
    assert!(
        html.contains(">80<"),
        "Report must display total file count (80)"
    );

    // Must contain total data size (formatted)
    // 125 GB ≈ "116.4 GB" or similar formatting
    assert!(
        html.contains("GB"),
        "Report must show data size in human-readable format"
    );

    // Must contain job names
    assert!(
        html.contains("Card A Offload"),
        "Report must list job names"
    );
    assert!(
        html.contains("Card B Offload"),
        "Report must list job names"
    );

    // Must contain source paths
    assert!(
        html.contains("/Volumes/LUMIX/DCIM"),
        "Report must show source paths"
    );

    // Must contain status indicators
    assert!(html.contains("completed"), "Report must show job statuses");

    println!("[AC-5.1] PASS: Day report HTML contains complete statistics");
}

/// AC-5.2: Job report HTML contains per-file hash details
#[test]
fn ac5_job_report_html_per_file_hashes() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = setup_db(&db_path);

    // Create a job with tasks
    checkpoint::create_job(
        &conn,
        "test-job-001",
        "AC5 Test Offload",
        "/Volumes/LUMIX/DCIM",
        None,
    )
    .unwrap();

    // Add copy tasks with hash values
    let files = [
        (
            "A001C0001.mov",
            1073741824u64,
            "abc123def456",
            "e3b0c44298fc1c14",
        ),
        (
            "A001C0002.mov",
            536870912,
            "789xyz000111",
            "9f86d081884c7d65",
        ),
        (
            "A001C0003.mov",
            268435456,
            "feedfacecafe",
            "a591a6d40bf42040",
        ),
    ];

    for (idx, (name, size, xxh, sha)) in files.iter().enumerate() {
        let source = format!("/Volumes/LUMIX/DCIM/{}", name);
        let dest = format!("/Volumes/A001/DIT_TEST/{}", name);
        let task_id = format!("task-{:03}", idx);
        checkpoint::insert_task(&conn, &task_id, "test-job-001", &source, &dest, *size).unwrap();

        // Mark completed with hashes
        conn.execute(
            "UPDATE copy_tasks SET status = 'completed', hash_xxh64 = ?1, hash_sha256 = ?2 WHERE source_path = ?3 AND job_id = 'test-job-001'",
            rusqlite::params![xxh, sha, source],
        )
        .unwrap();
    }

    // Update job status
    conn.execute(
        "UPDATE jobs SET status = 'completed' WHERE id = 'test-job-001'",
        [],
    )
    .unwrap();

    // Generate job report
    let job_report = report::get_job_report(&conn, "test-job-001").unwrap();
    let html = report::render_job_report_html(&job_report);

    // Must be valid HTML
    assert!(html.contains("<!DOCTYPE html>"), "Must be valid HTML5");

    // Must contain job name
    assert!(
        html.contains("AC5 Test Offload"),
        "Report must contain job name"
    );

    // Must contain file names
    assert!(
        html.contains("A001C0001.mov"),
        "Report must list file names"
    );
    assert!(
        html.contains("A001C0002.mov"),
        "Report must list file names"
    );
    assert!(
        html.contains("A001C0003.mov"),
        "Report must list file names"
    );

    // Must contain hash values
    assert!(
        html.contains("abc123def456"),
        "Report must show XXH64 hash values"
    );
    assert!(
        html.contains("e3b0c44298fc1c14"),
        "Report must show SHA256 hash values"
    );

    // Must contain file sizes
    assert!(
        html.contains("1073741824") || html.contains("1.0 GB") || html.contains("1,073,741,824"),
        "Report must show file sizes"
    );

    println!("[AC-5.2] PASS: Job report HTML contains per-file hash details");
}

/// AC-5.3: Report can be saved to disk as valid HTML file
#[test]
fn ac5_report_save_to_disk() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = setup_db(&db_path);

    // Create a minimal job
    checkpoint::create_job(&conn, "save-test", "Save Test Job", "/tmp/source", None).unwrap();
    conn.execute(
        "UPDATE jobs SET status = 'completed' WHERE id = 'save-test'",
        [],
    )
    .unwrap();

    // Get day report
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let day_report = report::get_day_report(&conn, &today).unwrap();
    let html = report::render_day_report_html(&day_report);

    // Save to disk
    let reports_dir = dir.path().join("reports");
    let result = report::save_report(&reports_dir, &today, &html);
    assert!(result.is_ok(), "Report must save successfully");

    let saved_path = result.unwrap();
    assert!(saved_path.exists(), "Saved report file must exist");

    // Read back and verify
    let content = std::fs::read_to_string(&saved_path).unwrap();
    assert!(
        content.starts_with("<!DOCTYPE html>"),
        "Saved file must be valid HTML"
    );
    assert!(
        content.len() > 500,
        "Report HTML must have substantial content"
    );

    println!("[AC-5.3] PASS: Report saved to disk as valid HTML file");
}

/// AC-5.4: Report from full offload contains accurate data
#[tokio::test]
async fn ac5_report_from_offload_accurate() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let dest = dir.path().join("dest");
    let db_path = dir.path().join("test.db");

    create_test_files(&source, 5, 8192);
    std::fs::create_dir_all(&dest).unwrap();

    let conn = setup_db(&db_path);
    let db = wrap_db(conn);

    let config = OffloadConfig {
        source_path: source.clone(),
        dest_paths: vec![dest.clone()],
        hash_algorithms: vec![HashAlgorithm::XXH64],
        source_verify: false,
        post_verify: true,
        generate_mhl: false,
        cascade: false,
        ..Default::default()
    };

    let (tx, _rx) = mpsc::unbounded_channel::<OffloadEvent>();
    let wf = OffloadWorkflow::new(config, db.clone(), tx);
    let result = wf.execute().await.unwrap();
    assert!(result.success);

    // Get the job ID from DB
    let conn = db.lock().unwrap();
    let job_id: String = conn
        .query_row(
            "SELECT id FROM jobs ORDER BY created_at DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();

    // Get job report
    let job_report = report::get_job_report(&conn, &job_id).unwrap();

    // Verify accuracy
    assert_eq!(
        job_report.summary.total_files, 5,
        "Report must show correct file count"
    );
    assert_eq!(
        job_report.summary.completed_files, 5,
        "Report must show all files completed"
    );
    assert_eq!(
        job_report.summary.failed_files, 0,
        "Report must show zero failures"
    );
    assert_eq!(
        job_report.tasks.len(),
        5,
        "Report must contain task details for all files"
    );

    // Verify each task has a hash
    for task in &job_report.tasks {
        assert_eq!(task.status, "completed", "All tasks must be completed");
        assert!(
            task.hash_xxh64.is_some(),
            "Each task must have XXH64 hash: {}",
            task.source_path
        );
    }

    // Verify HTML rendering
    let html = report::render_job_report_html(&job_report);
    assert!(html.contains("<!DOCTYPE html>"));
    for i in 1..=5 {
        let filename = format!("A001C{:04}.mov", i);
        assert!(
            html.contains(&filename),
            "HTML report must contain file: {}",
            filename
        );
    }

    println!("[AC-5.4] PASS: Report from offload contains accurate data with per-file hashes");
}

// ═══════════════════════════════════════════════════════════════════════════
// AC-7: MHL App-Integrated Verification
// ═══════════════════════════════════════════════════════════════════════════

/// AC-7.1: MHL verification request contract is available to the app
#[test]
fn ac7_app_verifier_request_contract() {
    let options = MhlVerifyOptions {
        chain_only: true,
        verify_all_generations: false,
        generation: Some(2),
    };
    let json = serde_json::to_value(options).unwrap();

    assert_eq!(json["chainOnly"], true);
    assert_eq!(json["verifyAllGenerations"], false);
    assert_eq!(json["generation"], 2);

    println!("[AC-7.1] PASS: MHL verifier request contract is app-compatible");
}

/// AC-7.2: The app verifier can verify a programmatically created MHL directory
#[tokio::test]
async fn ac7_app_verifier_verifies_mhl_directory() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("VerifyTestMedia");
    let clips_dir = root.join("Clips");
    std::fs::create_dir_all(&clips_dir).unwrap();

    // Create test files with known content
    let files = vec![
        ("Clips/test1.mov", b"test video content one" as &[u8]),
        ("Clips/test2.mov", b"test video content two" as &[u8]),
        ("Clips/test3.mov", b"test video content three" as &[u8]),
    ];

    for (rel_path, data) in &files {
        let full_path = root.join(rel_path);
        std::fs::write(&full_path, data).unwrap();
    }

    // Compute real hashes
    let mut file_hashes = HashMap::new();
    let mut file_metadata = HashMap::new();
    let now = chrono::Utc::now();

    for (rel_path, data) in &files {
        use xxhash_rust::xxh64::Xxh64;
        let mut hasher = Xxh64::new(0);
        hasher.update(data);
        let hex = format!("{:016x}", hasher.digest());

        file_hashes.insert(
            rel_path.to_string(),
            vec![HashResult {
                algorithm: HashAlgorithm::XXH64,
                hex_digest: hex,
            }],
        );
        file_metadata.insert(rel_path.to_string(), (data.len() as u64, now));
    }

    // Create MHL generation
    let mut history = mhl::load_or_create_history(&root).await.unwrap();
    mhl::create_generation(
        &mut history,
        &file_hashes,
        &file_metadata,
        MhlProcessType::Transfer,
        &MhlConfig::default(),
    )
    .await
    .unwrap();

    // Verify the MHL structure is correct for the app verifier to parse
    let ascmhl_dir = root.join(ASCMHL_DIR_NAME);
    assert!(ascmhl_dir.exists(), "ascmhl directory must exist");
    assert!(
        ascmhl_dir.join(CHAIN_FILE_NAME).exists(),
        "chain file must exist"
    );

    // Verify chain integrity programmatically
    let chain_results = mhl::verify_chain(&history).await.unwrap();
    assert_eq!(chain_results.len(), 1);
    assert!(chain_results[0].1, "Chain entry must be valid");

    // Read the manifest and verify it can be parsed back
    let manifest_path = ascmhl_dir.join(&history.chain[0].path);
    let xml = std::fs::read_to_string(&manifest_path).unwrap();

    // The manifest must be parseable by the app verifier.
    assert!(xml.contains("<hashlist version=\"2.0\""));
    assert!(xml.contains("<hashes>"));
    assert!(xml.contains("<hash>"));
    assert!(xml.contains("<path size="));
    assert!(xml.contains("<xxh64 action="));

    // Verify all files are in the manifest
    for (rel_path, _) in &files {
        assert!(
            xml.contains(rel_path),
            "Manifest must contain file: {}",
            rel_path
        );
    }

    let report = verify_mhl_path(&root, MhlVerifyOptions::default()).unwrap();
    assert!(report.summary.success);
    assert_eq!(report.summary.chain_valid, 1);
    assert_eq!(report.summary.passed, files.len());
    assert!(report.issues.is_empty());

    println!("[AC-7.2] PASS: MHL directory created by DIT Pro is app-verifiable");
}

/// AC-7.3: MHL tamper detection works (chain integrity check)
#[tokio::test]
async fn ac7_tamper_detection() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("TamperTestMedia");
    create_test_files(&root, 3, 256);

    let mut history = mhl::load_or_create_history(&root).await.unwrap();
    let now = chrono::Utc::now();

    let mut file_hashes = HashMap::new();
    let mut file_metadata = HashMap::new();
    for i in 1..=3 {
        let name = format!("A001C{:04}.mov", i);
        file_hashes.insert(
            name.clone(),
            vec![HashResult {
                algorithm: HashAlgorithm::XXH64,
                hex_digest: format!("{:016x}", i * 99999),
            }],
        );
        file_metadata.insert(name, (256u64, now));
    }

    let manifest_path = mhl::create_generation(
        &mut history,
        &file_hashes,
        &file_metadata,
        MhlProcessType::Transfer,
        &MhlConfig::default(),
    )
    .await
    .unwrap();

    // Verify chain is valid before tampering
    let results_before = mhl::verify_chain(&history).await.unwrap();
    assert!(results_before[0].1, "Chain must be valid before tampering");

    // Tamper with the manifest file
    let mut content = std::fs::read_to_string(&manifest_path).unwrap();
    content = content.replace("A001C0001.mov", "TAMPERED.mov");
    std::fs::write(&manifest_path, &content).unwrap();

    // Chain verification must detect the tampering
    let results_after = mhl::verify_chain(&history).await.unwrap();
    assert!(
        !results_after[0].1,
        "Chain must detect tampering after manifest modification"
    );

    // Restore original and verify it passes again
    content = content.replace("TAMPERED.mov", "A001C0001.mov");
    std::fs::write(&manifest_path, &content).unwrap();
    let results_restored = mhl::verify_chain(&history).await.unwrap();
    assert!(
        results_restored[0].1,
        "Chain must pass after restoring original content"
    );

    println!("[AC-7.3] PASS: Tamper detection correctly identifies manifest modifications");
}

/// AC-7.4: MHL cross-verification — DIT engine hashes match CLI hashes
#[test]
fn ac7_hash_cross_verification() {
    use app_lib::hash_engine;

    let dir = tempfile::tempdir().unwrap();
    let test_file = dir.path().join("cross_verify.bin");

    // Create a test file with known content
    let content = b"The quick brown fox jumps over the lazy dog";
    std::fs::write(&test_file, content).unwrap();

    // Compute hash using DIT hash engine
    let rt = tokio::runtime::Runtime::new().unwrap();
    let hash_config = HashEngineConfig {
        algorithms: vec![HashAlgorithm::XXH64, HashAlgorithm::SHA256],
        buffer_size: 4 * 1024 * 1024,
    };
    let dit_results = rt
        .block_on(hash_engine::hash_file(&test_file, &hash_config))
        .unwrap();

    let dit_xxh64 = dit_results
        .iter()
        .find(|r| r.algorithm == HashAlgorithm::XXH64)
        .unwrap();
    let dit_sha256 = dit_results
        .iter()
        .find(|r| r.algorithm == HashAlgorithm::SHA256)
        .unwrap();

    // Compute hash independently (as the CLI would)
    use sha2::Digest;
    use xxhash_rust::xxh64::Xxh64;

    let data = std::fs::read(&test_file).unwrap();

    let mut xxh = Xxh64::new(0);
    xxh.update(&data);
    let independent_xxh64 = format!("{:016x}", xxh.digest());

    let mut sha = sha2::Sha256::new();
    sha.update(&data);
    let independent_sha256 = format!("{:x}", sha.finalize());

    assert_eq!(
        dit_xxh64.hex_digest, independent_xxh64,
        "DIT engine XXH64 must match independent computation"
    );
    assert_eq!(
        dit_sha256.hex_digest, independent_sha256,
        "DIT engine SHA256 must match independent computation"
    );

    println!("[AC-7.4] PASS: DIT engine hashes match independent computation (CLI-compatible)");
}
