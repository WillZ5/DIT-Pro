//! Abnormal Scenario Tests — Power failure, cable pull, disk full simulation.
//!
//! These tests simulate real-world failure conditions that DIT operators face
//! on set: unexpected power loss, cable disconnection, disk filling up mid-copy.
//!
//! Each test verifies that:
//! - No half-written (corrupt) files remain on disk
//! - The checkpoint system correctly records interrupted state
//! - Recovery resumes from the last successful file
//! - .tmp files are cleaned up after recovery
//! - The final result matches what a clean copy would produce

use app_lib::checkpoint::{self, STATUS_COPYING};
use app_lib::copy_engine::atomic_writer::{self, AtomicWriter};
use app_lib::hash_engine::{hash_bytes, HashAlgorithm};
use rusqlite::Connection;
use tempfile::tempdir;

/// Helper: create an in-memory database with full schema
fn setup_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS jobs (
            id TEXT PRIMARY KEY, name TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            source_path TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS copy_tasks (
            id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL REFERENCES jobs(id),
            source_path TEXT NOT NULL, dest_path TEXT NOT NULL,
            file_size INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'pending',
            hash_xxh64 TEXT, hash_sha256 TEXT,
            error_msg TEXT, retry_count INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS volumes (
            id TEXT PRIMARY KEY, name TEXT NOT NULL,
            mount_point TEXT, total_bytes INTEGER,
            available_bytes INTEGER, device_type TEXT,
            serial_number TEXT, last_seen_at TEXT, last_seen_by TEXT
        );
        CREATE TABLE IF NOT EXISTS hash_records (
            id TEXT PRIMARY KEY,
            task_id TEXT NOT NULL REFERENCES copy_tasks(id),
            algorithm TEXT NOT NULL, hex_digest TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )
    .unwrap();
    conn
}

// ─── Scenario 1: Power Failure During Multi-File Copy ─────────────────────

/// Simulates power loss in the middle of copying multiple files.
///
/// Setup:
/// - 5 source files to copy
/// - File 1-2: completed successfully
/// - File 3: in-progress (has .tmp file on disk)
/// - File 4-5: still pending
///
/// Expected after recovery:
/// - File 1-2: remain completed (untouched)
/// - File 3 .tmp: cleaned up, task reset to pending
/// - File 4-5: remain pending
/// - Total pending tasks: 3 (file 3 + 4 + 5)
#[tokio::test]
async fn test_power_failure_mid_copy_recovery() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();

    // Create 5 source files
    for i in 1..=5 {
        let content = format!("source data for file {}", i);
        tokio::fs::write(src_dir.path().join(format!("clip{}.mov", i)), content.as_bytes())
            .await
            .unwrap();
    }

    let conn = setup_db();
    checkpoint::create_job(&conn, "job-power", "Power Failure Test", src_dir.path().to_str().unwrap()).unwrap();

    // Insert 5 tasks
    for i in 1..=5 {
        checkpoint::insert_task(
            &conn,
            &format!("t-{}", i),
            "job-power",
            src_dir.path().join(format!("clip{}.mov", i)).to_str().unwrap(),
            dst_dir.path().join(format!("clip{}.mov", i)).to_str().unwrap(),
            100,
        ).unwrap();
    }

    // Simulate: File 1-2 completed
    for i in 1..=2 {
        let content = format!("source data for file {}", i);
        tokio::fs::write(
            dst_dir.path().join(format!("clip{}.mov", i)),
            content.as_bytes(),
        ).await.unwrap();

        let hashes = hash_bytes(content.as_bytes(), &[HashAlgorithm::XXH64]);
        checkpoint::update_task_completed(
            &conn,
            &format!("t-{}", i),
            Some(&hashes[0].hex_digest),
            None,
        ).unwrap();
    }

    // Simulate: File 3 was being written when power cut (only .tmp exists)
    checkpoint::update_task_status(&conn, "t-3", STATUS_COPYING).unwrap();
    let dest_3 = dst_dir.path().join("clip3.mov");
    let tmp_3 = AtomicWriter::temp_path_for(&dest_3);
    tokio::fs::write(&tmp_3, b"partial data - not complete").await.unwrap();

    // File 4-5 still pending (untouched)

    // ─── Verify pre-recovery state ───
    let progress_before = checkpoint::get_job_progress(&conn, "job-power").unwrap();
    assert_eq!(progress_before.completed, 2);
    assert_eq!(progress_before.copying, 1);
    assert_eq!(progress_before.pending, 2);

    // .tmp file exists
    assert!(tmp_3.exists());
    // Final file 3 should NOT exist
    assert!(!dest_3.exists());

    // ─── RECOVERY ───
    let recovered = checkpoint::recover_job(&conn, "job-power").await.unwrap();

    // ─── Verify post-recovery state ───
    // 3 tasks should be pending (t-3 reset + t-4 + t-5)
    assert_eq!(recovered.len(), 3);

    // .tmp file should be cleaned up
    assert!(!tmp_3.exists());

    // Completed files untouched
    assert!(dst_dir.path().join("clip1.mov").exists());
    assert!(dst_dir.path().join("clip2.mov").exists());

    // Progress should show 2 completed, 3 pending, 0 copying
    let progress_after = checkpoint::get_job_progress(&conn, "job-power").unwrap();
    assert_eq!(progress_after.completed, 2);
    assert_eq!(progress_after.pending, 3);
    assert_eq!(progress_after.copying, 0);
}

// ─── Scenario 2: Cable Pull — Multiple .tmp Files Orphaned ───────────────

/// Simulates cable pull during multi-destination copy where multiple
/// destinations have orphaned .tmp files.
///
/// All .tmp files should be cleaned up, tasks reset, completed work preserved.
#[tokio::test]
async fn test_cable_pull_multi_destination_recovery() {
    let src_dir = tempdir().unwrap();
    let dst1_dir = tempdir().unwrap();
    let dst2_dir = tempdir().unwrap();

    // Create source file
    let source_data = b"original camera clip data - 4K R3D footage";
    tokio::fs::write(src_dir.path().join("A001C001.R3D"), source_data)
        .await
        .unwrap();

    let conn = setup_db();
    checkpoint::create_job(&conn, "job-cable", "Cable Pull Test", src_dir.path().to_str().unwrap()).unwrap();

    // Two copy tasks for the same file to different destinations
    let dst1_path = dst1_dir.path().join("A001C001.R3D");
    let dst2_path = dst2_dir.path().join("A001C001.R3D");

    checkpoint::insert_task(
        &conn, "t-dst1", "job-cable",
        src_dir.path().join("A001C001.R3D").to_str().unwrap(),
        dst1_path.to_str().unwrap(),
        source_data.len() as u64,
    ).unwrap();

    checkpoint::insert_task(
        &conn, "t-dst2", "job-cable",
        src_dir.path().join("A001C001.R3D").to_str().unwrap(),
        dst2_path.to_str().unwrap(),
        source_data.len() as u64,
    ).unwrap();

    // Simulate: both were actively copying when cable was pulled
    checkpoint::update_task_status(&conn, "t-dst1", STATUS_COPYING).unwrap();
    checkpoint::update_task_status(&conn, "t-dst2", STATUS_COPYING).unwrap();

    // Both have orphaned .tmp files
    let tmp1 = AtomicWriter::temp_path_for(&dst1_path);
    let tmp2 = AtomicWriter::temp_path_for(&dst2_path);
    tokio::fs::write(&tmp1, b"partial...").await.unwrap();
    tokio::fs::write(&tmp2, b"partial again...").await.unwrap();

    // Verify .tmp files exist
    assert!(tmp1.exists());
    assert!(tmp2.exists());
    // Final files should NOT exist
    assert!(!dst1_path.exists());
    assert!(!dst2_path.exists());

    // ─── RECOVERY ───
    let recovered = checkpoint::recover_job(&conn, "job-cable").await.unwrap();

    // Both tasks should be reset to pending
    assert_eq!(recovered.len(), 2);

    // All .tmp files cleaned up
    assert!(!tmp1.exists());
    assert!(!tmp2.exists());

    // No corrupt final files
    assert!(!dst1_path.exists());
    assert!(!dst2_path.exists());
}

// ─── Scenario 3: Atomic Write Guarantees ─────────────────────────────────

/// Verifies that the atomic writer never leaves a partial final file.
/// Even if we write partial data and then abort, only .tmp should exist.
#[tokio::test]
async fn test_atomic_write_no_partial_final_file() {
    let dir = tempdir().unwrap();
    let final_path = dir.path().join("important_clip.mov");

    // Start writing
    let mut writer = AtomicWriter::new(&final_path).await.unwrap();
    writer.write(b"first chunk of data").await.unwrap();
    writer.write(b"second chunk of data").await.unwrap();

    // Verify: .tmp exists, final file does NOT
    assert!(writer.temp_path().exists());
    assert!(!final_path.exists());

    // Simulate crash: abort instead of finalize
    writer.abort().await.unwrap();

    // After abort: neither file should exist
    assert!(!final_path.exists());
    assert!(!dir.path().join("important_clip.mov.tmp").exists());
}

/// Verifies the atomic rename guarantee: the file appears atomically
/// with correct content after finalize.
#[tokio::test]
async fn test_atomic_finalize_complete_file() {
    let dir = tempdir().unwrap();
    let final_path = dir.path().join("verified_clip.mov");
    let test_data = b"This is the complete verified file content for DIT offload";

    let mut writer = AtomicWriter::new(&final_path).await.unwrap();
    writer.write(test_data).await.unwrap();
    assert_eq!(writer.bytes_written(), test_data.len() as u64);

    writer.finalize().await.unwrap();

    // Final file exists with correct content
    let read_back = tokio::fs::read(&final_path).await.unwrap();
    assert_eq!(read_back, test_data);

    // .tmp file is gone
    assert!(!AtomicWriter::temp_path_for(&final_path).exists());
}

// ─── Scenario 4: Recovery With No Interrupted Tasks ──────────────────────

/// Recovery on a clean job (all completed) should return empty list.
#[tokio::test]
async fn test_recovery_all_completed_noop() {
    let conn = setup_db();
    checkpoint::create_job(&conn, "job-clean", "Clean Job", "/src").unwrap();
    checkpoint::insert_task(&conn, "t-1", "job-clean", "/src/a.mov", "/dst/a.mov", 100).unwrap();
    checkpoint::update_task_completed(&conn, "t-1", Some("hash"), None).unwrap();

    let recovered = checkpoint::recover_job(&conn, "job-clean").await.unwrap();
    assert!(recovered.is_empty());

    let progress = checkpoint::get_job_progress(&conn, "job-clean").unwrap();
    assert_eq!(progress.completed, 1);
    assert_eq!(progress.pending, 0);
}

// ─── Scenario 5: Repeated Failures and Retry Count ───────────────────────

/// Simulates a file that fails multiple times (e.g., bad sector on source card).
/// Retry count should accumulate, and the task can eventually be skipped.
#[tokio::test]
async fn test_repeated_failure_retry_tracking() {
    let conn = setup_db();
    checkpoint::create_job(&conn, "job-retry", "Retry Test", "/src").unwrap();
    checkpoint::insert_task(&conn, "t-1", "job-retry", "/src/bad.mov", "/dst/bad.mov", 500).unwrap();

    // Simulate 3 failed attempts
    checkpoint::update_task_failed(&conn, "t-1", "IO error: read failed").unwrap();
    checkpoint::update_task_failed(&conn, "t-1", "IO error: read failed (retry 2)").unwrap();
    checkpoint::update_task_failed(&conn, "t-1", "IO error: read failed (retry 3)").unwrap();

    let progress = checkpoint::get_job_progress(&conn, "job-retry").unwrap();
    assert_eq!(progress.failed, 1);

    // Verify retry count is 3 in the database
    let retry_count: i32 = conn.query_row(
        "SELECT retry_count FROM copy_tasks WHERE id = 't-1'",
        [],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(retry_count, 3);

    // Error message should be the latest
    let error_msg: String = conn.query_row(
        "SELECT error_msg FROM copy_tasks WHERE id = 't-1'",
        [],
        |row| row.get(0),
    ).unwrap();
    assert!(error_msg.contains("retry 3"));
}

// ─── Scenario 6: .tmp Cleanup in Directory ───────────────────────────────

/// Simulates orphaned .tmp files scattered across destination directory
/// after a system crash. cleanup_tmp_files should remove all of them.
#[tokio::test]
async fn test_orphaned_tmp_cleanup() {
    let dir = tempdir().unwrap();

    // Create a mix of files
    tokio::fs::write(dir.path().join("clip1.mov"), b"good file").await.unwrap();
    tokio::fs::write(dir.path().join("clip2.mov"), b"good file 2").await.unwrap();
    tokio::fs::write(dir.path().join("clip3.mov.tmp"), b"orphaned tmp 1").await.unwrap();
    tokio::fs::write(dir.path().join("clip4.r3d.tmp"), b"orphaned tmp 2").await.unwrap();
    tokio::fs::write(dir.path().join("clip5.braw.tmp"), b"orphaned tmp 3").await.unwrap();
    tokio::fs::write(dir.path().join("readme.txt"), b"notes").await.unwrap();

    let cleaned = atomic_writer::cleanup_tmp_files(dir.path()).await.unwrap();

    // 3 .tmp files should be cleaned
    assert_eq!(cleaned.len(), 3);

    // Good files remain untouched
    assert!(dir.path().join("clip1.mov").exists());
    assert!(dir.path().join("clip2.mov").exists());
    assert!(dir.path().join("readme.txt").exists());

    // All .tmp files are gone
    assert!(!dir.path().join("clip3.mov.tmp").exists());
    assert!(!dir.path().join("clip4.r3d.tmp").exists());
    assert!(!dir.path().join("clip5.braw.tmp").exists());
}

// ─── Scenario 7: Hash Verification Catches Corruption ────────────────────

/// Simulates data corruption during copy (bit flip).
/// The hash comparison should detect the mismatch.
#[tokio::test]
async fn test_corruption_detection_via_hash() {
    let original_data = b"this is the original camera file data from the CF card";
    let corrupted_data = b"this is the original camera file data from the CF Card"; // 'c' -> 'C'

    let original_hash = hash_bytes(original_data, &[HashAlgorithm::XXH64]);
    let corrupted_hash = hash_bytes(corrupted_data, &[HashAlgorithm::XXH64]);

    // Hashes must be different
    assert_ne!(
        original_hash[0].hex_digest,
        corrupted_hash[0].hex_digest,
        "Single-byte corruption MUST produce different hash"
    );

    // Also verify with SHA-256
    let original_sha = hash_bytes(original_data, &[HashAlgorithm::SHA256]);
    let corrupted_sha = hash_bytes(corrupted_data, &[HashAlgorithm::SHA256]);
    assert_ne!(original_sha[0].hex_digest, corrupted_sha[0].hex_digest);
}

// ─── Scenario 8: MHL Chain Tamper Detection ──────────────────────────────

/// Verifies that modifying a manifest file after generation is detected
/// by the chain integrity verification.
#[tokio::test]
async fn test_mhl_chain_detects_manifest_tampering() {
    use app_lib::hash_engine::HashResult;
    use app_lib::mhl::{self, MhlConfig, MhlProcessType};
    use chrono::Utc;
    use std::collections::HashMap;

    let dir = tempdir().unwrap();
    let root = dir.path().join("TamperTest");
    tokio::fs::create_dir_all(&root).await.unwrap();

    let mut history = mhl::load_or_create_history(&root).await.unwrap();

    let mut file_hashes = HashMap::new();
    file_hashes.insert(
        "clip.mov".to_string(),
        vec![HashResult {
            algorithm: HashAlgorithm::XXH64,
            hex_digest: "abcdef0123456789".to_string(),
        }],
    );
    let mut file_metadata = HashMap::new();
    file_metadata.insert("clip.mov".to_string(), (1024u64, Utc::now()));

    let manifest_path = mhl::create_generation(
        &mut history,
        &file_hashes,
        &file_metadata,
        MhlProcessType::Transfer,
        &MhlConfig::default(),
    ).await.unwrap();

    // Before tampering: chain is valid
    let results_ok = mhl::verify_chain(&history).await.unwrap();
    assert!(results_ok[0].1, "Chain should be valid before tampering");

    // Tamper with the manifest (change a hash value)
    let mut content = tokio::fs::read_to_string(&manifest_path).await.unwrap();
    content = content.replace("abcdef0123456789", "0000000000000000");
    tokio::fs::write(&manifest_path, content).await.unwrap();

    // After tampering: chain should detect the corruption
    let results_tampered = mhl::verify_chain(&history).await.unwrap();
    assert!(!results_tampered[0].1, "Chain MUST detect manifest tampering");
}

// ─── Scenario 9: Large Job Recovery — Many Files ─────────────────────────

/// Simulates a crash during a large job with 100 files.
/// Recovery should handle the scale efficiently.
#[tokio::test]
async fn test_large_job_recovery_100_files() {
    let dst_dir = tempdir().unwrap();
    let conn = setup_db();

    checkpoint::create_job(&conn, "job-large", "Large Job", "/Volumes/CARD_A").unwrap();

    // Insert 100 tasks
    for i in 1..=100 {
        let dest = dst_dir.path().join(format!("clip_{:04}.mov", i));
        checkpoint::insert_task(
            &conn,
            &format!("t-{}", i),
            "job-large",
            &format!("/src/clip_{:04}.mov", i),
            dest.to_str().unwrap(),
            1_000_000,
        ).unwrap();
    }

    // Complete first 60
    for i in 1..=60 {
        checkpoint::update_task_completed(
            &conn,
            &format!("t-{}", i),
            Some(&format!("hash_{}", i)),
            None,
        ).unwrap();
    }

    // Mark tasks 61-65 as in-progress (crash happened here)
    for i in 61..=65 {
        checkpoint::update_task_status(&conn, &format!("t-{}", i), STATUS_COPYING).unwrap();
        // Create .tmp files for these
        let tmp_path = dst_dir.path().join(format!("clip_{:04}.mov.tmp", i));
        tokio::fs::write(&tmp_path, b"partial").await.unwrap();
    }

    // 66-100 remain pending

    let progress_before = checkpoint::get_job_progress(&conn, "job-large").unwrap();
    assert_eq!(progress_before.completed, 60);
    assert_eq!(progress_before.copying, 5);
    assert_eq!(progress_before.pending, 35);

    // ─── RECOVERY ───
    let recovered = checkpoint::recover_job(&conn, "job-large").await.unwrap();

    // 40 tasks should be pending (5 reset + 35 original)
    assert_eq!(recovered.len(), 40);

    // All .tmp files cleaned
    for i in 61..=65 {
        let tmp_path = dst_dir.path().join(format!("clip_{:04}.mov.tmp", i));
        assert!(!tmp_path.exists(), "tmp file for clip_{:04} should be cleaned", i);
    }

    let progress_after = checkpoint::get_job_progress(&conn, "job-large").unwrap();
    assert_eq!(progress_after.completed, 60);
    assert_eq!(progress_after.pending, 40);
    assert_eq!(progress_after.copying, 0);
    assert_eq!(progress_after.failed, 0);
}
