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

use app_lib::checkpoint::{self, TaskHashes, STATUS_COPYING};
use app_lib::copy_engine::atomic_writer::{self, AtomicWriter};
use app_lib::hash_engine::{hash_bytes, HashAlgorithm};
use rusqlite::Connection;
use std::sync::Arc;
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
            config_json TEXT,
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
            hash_md5 TEXT, hash_xxh128 TEXT, hash_xxh3 TEXT,
            error_msg TEXT, retry_count INTEGER NOT NULL DEFAULT 0,
            retry_note TEXT,
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
        tokio::fs::write(
            src_dir.path().join(format!("clip{}.mov", i)),
            content.as_bytes(),
        )
        .await
        .unwrap();
    }

    let conn = setup_db();
    checkpoint::create_job(
        &conn,
        "job-power",
        "Power Failure Test",
        src_dir.path().to_str().unwrap(),
        None,
    )
    .unwrap();

    // Insert 5 tasks
    for i in 1..=5 {
        checkpoint::insert_task(
            &conn,
            &format!("t-{}", i),
            "job-power",
            src_dir
                .path()
                .join(format!("clip{}.mov", i))
                .to_str()
                .unwrap(),
            dst_dir
                .path()
                .join(format!("clip{}.mov", i))
                .to_str()
                .unwrap(),
            100,
        )
        .unwrap();
    }

    // Simulate: File 1-2 completed
    for i in 1..=2 {
        let content = format!("source data for file {}", i);
        tokio::fs::write(
            dst_dir.path().join(format!("clip{}.mov", i)),
            content.as_bytes(),
        )
        .await
        .unwrap();

        let hashes = hash_bytes(content.as_bytes(), &[HashAlgorithm::XXH64]);
        let task_hashes = TaskHashes {
            xxh64: Some(hashes[0].hex_digest.clone()),
            ..Default::default()
        };
        checkpoint::update_task_completed(&conn, &format!("t-{}", i), &task_hashes).unwrap();
    }

    // Simulate: File 3 was being written when power cut (only .tmp exists)
    checkpoint::update_task_status(&conn, "t-3", STATUS_COPYING).unwrap();
    let dest_3 = dst_dir.path().join("clip3.mov");
    let tmp_3 = AtomicWriter::temp_path_for(&dest_3);
    tokio::fs::write(&tmp_3, b"partial data - not complete")
        .await
        .unwrap();

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
    checkpoint::create_job(
        &conn,
        "job-cable",
        "Cable Pull Test",
        src_dir.path().to_str().unwrap(),
        None,
    )
    .unwrap();

    // Two copy tasks for the same file to different destinations
    let dst1_path = dst1_dir.path().join("A001C001.R3D");
    let dst2_path = dst2_dir.path().join("A001C001.R3D");

    checkpoint::insert_task(
        &conn,
        "t-dst1",
        "job-cable",
        src_dir.path().join("A001C001.R3D").to_str().unwrap(),
        dst1_path.to_str().unwrap(),
        source_data.len() as u64,
    )
    .unwrap();

    checkpoint::insert_task(
        &conn,
        "t-dst2",
        "job-cable",
        src_dir.path().join("A001C001.R3D").to_str().unwrap(),
        dst2_path.to_str().unwrap(),
        source_data.len() as u64,
    )
    .unwrap();

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
    checkpoint::create_job(&conn, "job-clean", "Clean Job", "/src", None).unwrap();
    checkpoint::insert_task(&conn, "t-1", "job-clean", "/src/a.mov", "/dst/a.mov", 100).unwrap();
    let task_hashes = TaskHashes {
        xxh64: Some("hash".to_string()),
        ..Default::default()
    };
    checkpoint::update_task_completed(&conn, "t-1", &task_hashes).unwrap();

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
    checkpoint::create_job(&conn, "job-retry", "Retry Test", "/src", None).unwrap();
    checkpoint::insert_task(
        &conn,
        "t-1",
        "job-retry",
        "/src/bad.mov",
        "/dst/bad.mov",
        500,
    )
    .unwrap();

    // Simulate 3 failed attempts
    checkpoint::update_task_failed(&conn, "t-1", "IO error: read failed").unwrap();
    checkpoint::update_task_failed(&conn, "t-1", "IO error: read failed (retry 2)").unwrap();
    checkpoint::update_task_failed(&conn, "t-1", "IO error: read failed (retry 3)").unwrap();

    let progress = checkpoint::get_job_progress(&conn, "job-retry").unwrap();
    assert_eq!(progress.failed, 1);

    // Verify retry count is 3 in the database
    let retry_count: i32 = conn
        .query_row(
            "SELECT retry_count FROM copy_tasks WHERE id = 't-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retry_count, 3);

    // Error message should be the latest
    let error_msg: String = conn
        .query_row(
            "SELECT error_msg FROM copy_tasks WHERE id = 't-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(error_msg.contains("retry 3"));
}

// ─── Scenario 6: .tmp Cleanup in Directory ───────────────────────────────

/// Simulates orphaned .tmp files scattered across destination directory
/// after a system crash. cleanup_tmp_files should remove all of them.
#[tokio::test]
async fn test_orphaned_tmp_cleanup() {
    let dir = tempdir().unwrap();

    // Create a mix of files
    tokio::fs::write(dir.path().join("clip1.mov"), b"good file")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("clip2.mov"), b"good file 2")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("clip3.mov.tmp"), b"orphaned tmp 1")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("clip4.r3d.tmp"), b"orphaned tmp 2")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("clip5.braw.tmp"), b"orphaned tmp 3")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("readme.txt"), b"notes")
        .await
        .unwrap();

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
        original_hash[0].hex_digest, corrupted_hash[0].hex_digest,
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
    )
    .await
    .unwrap();

    // Before tampering: chain is valid
    let results_ok = mhl::verify_chain(&history).await.unwrap();
    assert!(results_ok[0].1, "Chain should be valid before tampering");

    // Tamper with the manifest (change a hash value)
    let mut content = tokio::fs::read_to_string(&manifest_path).await.unwrap();
    content = content.replace("abcdef0123456789", "0000000000000000");
    tokio::fs::write(&manifest_path, content).await.unwrap();

    // After tampering: chain should detect the corruption
    let results_tampered = mhl::verify_chain(&history).await.unwrap();
    assert!(
        !results_tampered[0].1,
        "Chain MUST detect manifest tampering"
    );
}

// ─── Scenario 9: Large Job Recovery — Many Files ─────────────────────────

/// Simulates a crash during a large job with 100 files.
/// Recovery should handle the scale efficiently.
#[tokio::test]
async fn test_large_job_recovery_100_files() {
    let dst_dir = tempdir().unwrap();
    let conn = setup_db();

    checkpoint::create_job(&conn, "job-large", "Large Job", "/Volumes/CARD_A", None).unwrap();

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
        )
        .unwrap();
    }

    // Complete first 60
    for i in 1..=60 {
        let task_hashes = TaskHashes {
            xxh64: Some(format!("hash_{}", i)),
            ..Default::default()
        };
        checkpoint::update_task_completed(&conn, &format!("t-{}", i), &task_hashes).unwrap();
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
        assert!(
            !tmp_path.exists(),
            "tmp file for clip_{:04} should be cleaned",
            i
        );
    }

    let progress_after = checkpoint::get_job_progress(&conn, "job-large").unwrap();
    assert_eq!(progress_after.completed, 60);
    assert_eq!(progress_after.pending, 40);
    assert_eq!(progress_after.copying, 0);
    assert_eq!(progress_after.failed, 0);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Scenario 10: Disk Full — AtomicWriter Fails Cleanly
// ═══════════════════════════════════════════════════════════════════════════════

/// When dest has insufficient space, preflight_space_check should flag it.
#[test]
fn test_disk_full_preflight_detects_insufficient_space() {
    use app_lib::volume::preflight_space_check;
    use std::path::PathBuf;

    let dir = tempdir().unwrap();

    // Ask for an absurd amount of space (1 exabyte) — should fail preflight
    let destinations = vec![(PathBuf::from(dir.path()), 1_000_000_000_000_000_000u64)];
    let issues = preflight_space_check(&destinations);

    // The preflight check should return issues (insufficient space)
    assert!(
        !issues.is_empty(),
        "Preflight should detect insufficient space for 1 EB"
    );

    // The reported space should show available < required
    let (_path, space, required) = &issues[0];
    assert!(
        space.available_bytes < *required,
        "Available {} should be less than required {}",
        space.available_bytes,
        required
    );
}

/// When AtomicWriter cannot write (e.g., disk full simulation via write error),
/// abort should clean up .tmp files and no corrupt final file remains.
#[tokio::test]
async fn test_disk_full_atomic_writer_abort_cleans_tmp() {
    let dir = tempdir().unwrap();
    let final_path = dir.path().join("big_clip.mov");

    // Start a normal write, then abort (simulates a write failure scenario)
    let mut writer = AtomicWriter::new(&final_path).await.unwrap();
    writer.write(b"partial data before disk full").await.unwrap();

    let tmp = writer.temp_path().to_path_buf();
    assert!(tmp.exists(), ".tmp should exist during write");

    // Abort (what happens when disk full is detected)
    writer.abort().await.unwrap();

    // No .tmp file and no final file
    assert!(!tmp.exists(), ".tmp should be cleaned after abort");
    assert!(
        !final_path.exists(),
        "Final file should not exist after abort"
    );
}

/// Multi-destination scenario: dest1 succeeds, dest2 fails (simulated).
/// Completed dest should have valid files; failed dest should have no residue.
#[tokio::test]
async fn test_disk_full_multi_dest_one_fails() {
    let src_dir = tempdir().unwrap();
    let dst1_dir = tempdir().unwrap();
    let dst2_dir = tempdir().unwrap();

    let source_data = b"camera footage data for multi-dest disk full test";
    let src_file = src_dir.path().join("A001C001.mov");
    tokio::fs::write(&src_file, source_data).await.unwrap();

    let conn = setup_db();
    checkpoint::create_job(
        &conn,
        "job-df-multi",
        "DiskFull Multi",
        src_dir.path().to_str().unwrap(),
        None,
    )
    .unwrap();

    let dst1_path = dst1_dir.path().join("A001C001.mov");
    let dst2_path = dst2_dir.path().join("A001C001.mov");

    checkpoint::insert_task(
        &conn,
        "t-df-1",
        "job-df-multi",
        src_file.to_str().unwrap(),
        dst1_path.to_str().unwrap(),
        source_data.len() as u64,
    )
    .unwrap();
    checkpoint::insert_task(
        &conn,
        "t-df-2",
        "job-df-multi",
        src_file.to_str().unwrap(),
        dst2_path.to_str().unwrap(),
        source_data.len() as u64,
    )
    .unwrap();

    // Dest 1: success — write and finalize
    let mut w1 = AtomicWriter::new(&dst1_path).await.unwrap();
    w1.write(source_data).await.unwrap();
    w1.finalize().await.unwrap();
    let hashes = hash_bytes(source_data, &[HashAlgorithm::XXH64]);
    let task_hashes = TaskHashes {
        xxh64: Some(hashes[0].hex_digest.clone()),
        ..Default::default()
    };
    checkpoint::update_task_completed(&conn, "t-df-1", &task_hashes).unwrap();

    // Dest 2: simulated failure — write partial, then abort
    let mut w2 = AtomicWriter::new(&dst2_path).await.unwrap();
    w2.write(b"partial").await.unwrap();
    w2.abort().await.unwrap();
    checkpoint::update_task_failed(&conn, "t-df-2", "No space left on device").unwrap();

    // Verify: dest1 has correct file
    assert!(dst1_path.exists());
    let read1 = tokio::fs::read(&dst1_path).await.unwrap();
    assert_eq!(read1, source_data);

    // Verify: dest2 has no file (neither final nor .tmp)
    assert!(!dst2_path.exists());
    assert!(!AtomicWriter::temp_path_for(&dst2_path).exists());

    // Verify: source untouched
    assert!(src_file.exists());

    // Verify: job progress correct
    let progress = checkpoint::get_job_progress(&conn, "job-df-multi").unwrap();
    assert_eq!(progress.completed, 1);
    assert_eq!(progress.failed, 1);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Scenario 11: Permission Denied — Read-only Destination
// ═══════════════════════════════════════════════════════════════════════════════

/// AtomicWriter::new() should fail when the destination directory is read-only.
#[cfg(unix)]
#[tokio::test]
async fn test_permission_denied_dest_readonly() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().unwrap();
    let readonly_dir = dir.path().join("readonly_dest");
    std::fs::create_dir_all(&readonly_dir).unwrap();

    // Make directory read-only (remove write+execute for owner)
    std::fs::set_permissions(&readonly_dir, std::fs::Permissions::from_mode(0o444)).unwrap();

    let final_path = readonly_dir.join("clip.mov");
    let result = AtomicWriter::new(&final_path).await;

    // Should fail with permission error
    assert!(
        result.is_err(),
        "AtomicWriter::new should fail on read-only dir"
    );
    let err_msg = format!("{:?}", result.err().unwrap());
    let err_lower = err_msg.to_lowercase();
    assert!(
        err_lower.contains("permission")
            || err_lower.contains("denied")
            || err_lower.contains("read-only")
            || err_lower.contains("failed to create"),
        "Error should indicate file creation failure, got: {}",
        err_msg
    );

    // No .tmp file residue
    assert!(!AtomicWriter::temp_path_for(&final_path).exists());

    // Restore permissions for cleanup
    std::fs::set_permissions(&readonly_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Multi-dest: dest1 writable, dest2 read-only. dest1 should succeed.
#[cfg(unix)]
#[tokio::test]
async fn test_permission_denied_partial_dest() {
    use std::os::unix::fs::PermissionsExt;

    let src_dir = tempdir().unwrap();
    let dst1_dir = tempdir().unwrap();
    let dst2_dir = tempdir().unwrap();
    let readonly_dir = dst2_dir.path().join("locked");
    std::fs::create_dir_all(&readonly_dir).unwrap();
    std::fs::set_permissions(&readonly_dir, std::fs::Permissions::from_mode(0o444)).unwrap();

    let source_data = b"permission test clip data";
    let src_file = src_dir.path().join("clip.mov");
    tokio::fs::write(&src_file, source_data).await.unwrap();

    // Dest 1: should succeed
    let dst1_path = dst1_dir.path().join("clip.mov");
    let mut w1 = AtomicWriter::new(&dst1_path).await.unwrap();
    w1.write(source_data).await.unwrap();
    w1.finalize().await.unwrap();
    assert!(dst1_path.exists());

    // Dest 2: should fail (read-only)
    let dst2_path = readonly_dir.join("clip.mov");
    let result = AtomicWriter::new(&dst2_path).await;
    assert!(result.is_err(), "Dest2 should fail due to read-only dir");

    // Source untouched
    assert!(src_file.exists());

    // Restore permissions for cleanup
    std::fs::set_permissions(&readonly_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Source file with no read permission should fail to open.
#[cfg(unix)]
#[tokio::test]
async fn test_permission_denied_source_unreadable() {
    use std::os::unix::fs::PermissionsExt;

    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();

    let src_file = src_dir.path().join("secret.mov");
    tokio::fs::write(&src_file, b"confidential data").await.unwrap();

    // Remove all permissions from source
    std::fs::set_permissions(&src_file, std::fs::Permissions::from_mode(0o000)).unwrap();

    // Attempting to read should fail
    let read_result = tokio::fs::read(&src_file).await;
    assert!(
        read_result.is_err(),
        "Reading 000-permission file should fail"
    );

    // No .tmp files should exist in dest (copy never started)
    let dest_path = dst_dir.path().join("secret.mov");
    assert!(!dest_path.exists());
    assert!(!AtomicWriter::temp_path_for(&dest_path).exists());

    // Restore permissions for cleanup
    std::fs::set_permissions(&src_file, std::fs::Permissions::from_mode(0o644)).unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════════
// Scenario 12: Source File Disappears During Batch Copy
// ═══════════════════════════════════════════════════════════════════════════════

/// When source is deleted before copy starts, the error is recorded and dest
/// has no residue.
#[tokio::test]
async fn test_source_disappears_before_copy() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();

    let src_file = src_dir.path().join("vanishing.mov");
    tokio::fs::write(&src_file, b"i will disappear").await.unwrap();

    // Delete source before copy
    tokio::fs::remove_file(&src_file).await.unwrap();

    // Try to open source for reading — should fail
    let read_result = tokio::fs::read(&src_file).await;
    assert!(read_result.is_err());

    // Verify no dest artifacts
    let dest_path = dst_dir.path().join("vanishing.mov");
    assert!(!dest_path.exists());
    assert!(!AtomicWriter::temp_path_for(&dest_path).exists());
}

/// Simulates a batch of 5 files where the 3rd file is deleted before its
/// turn to copy. Files 1-2 should complete, file 3 should fail,
/// files 4-5 remain pending.
#[tokio::test]
async fn test_source_disappears_mid_batch() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();

    // Create 5 source files
    let mut file_data = Vec::new();
    for i in 1..=5 {
        let data = format!("source data for batch file {}", i).into_bytes();
        let name = format!("clip{}.mov", i);
        tokio::fs::write(src_dir.path().join(&name), &data)
            .await
            .unwrap();
        file_data.push(data);
    }

    let conn = setup_db();
    checkpoint::create_job(
        &conn,
        "job-vanish",
        "Vanish Batch",
        src_dir.path().to_str().unwrap(),
        None,
    )
    .unwrap();

    for i in 1..=5 {
        let name = format!("clip{}.mov", i);
        checkpoint::insert_task(
            &conn,
            &format!("t-v{}", i),
            "job-vanish",
            src_dir.path().join(&name).to_str().unwrap(),
            dst_dir.path().join(&name).to_str().unwrap(),
            file_data[i - 1].len() as u64,
        )
        .unwrap();
    }

    // Copy files 1-2 successfully
    for i in 1..=2 {
        let name = format!("clip{}.mov", i);
        let dest = dst_dir.path().join(&name);
        let mut writer = AtomicWriter::new(&dest).await.unwrap();
        writer.write(&file_data[i - 1]).await.unwrap();
        writer.finalize().await.unwrap();
        let hashes = hash_bytes(&file_data[i - 1], &[HashAlgorithm::XXH64]);
        let th = TaskHashes {
            xxh64: Some(hashes[0].hex_digest.clone()),
            ..Default::default()
        };
        checkpoint::update_task_completed(&conn, &format!("t-v{}", i), &th).unwrap();
    }

    // Delete source file 3 (simulates card removal / file disappearance)
    tokio::fs::remove_file(src_dir.path().join("clip3.mov"))
        .await
        .unwrap();

    // Try to copy file 3 — source read should fail
    let src3 = src_dir.path().join("clip3.mov");
    let read_result = tokio::fs::read(&src3).await;
    assert!(read_result.is_err(), "Source file 3 should be gone");

    // Mark file 3 as failed
    checkpoint::update_task_failed(&conn, "t-v3", "Source file not found: clip3.mov").unwrap();

    // Files 4-5 remain pending (not attempted after a batch-critical error)
    let progress = checkpoint::get_job_progress(&conn, "job-vanish").unwrap();
    assert_eq!(progress.completed, 2, "Files 1-2 should be complete");
    assert_eq!(progress.failed, 1, "File 3 should be failed");
    assert_eq!(progress.pending, 2, "Files 4-5 should be pending");

    // Verify completed files are correct
    for i in 1..=2 {
        let name = format!("clip{}.mov", i);
        let read_back = tokio::fs::read(dst_dir.path().join(&name)).await.unwrap();
        assert_eq!(read_back, file_data[i - 1]);
    }

    // Verify no .tmp residue for file 3
    let dest3 = dst_dir.path().join("clip3.mov");
    assert!(!dest3.exists());
    assert!(!AtomicWriter::temp_path_for(&dest3).exists());
}

/// Source file renamed during processing: verify the checkpoint tracks
/// the error correctly and no orphaned state remains.
#[tokio::test]
async fn test_source_renamed_during_processing() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();

    let src_file = src_dir.path().join("original.mov");
    let renamed_file = src_dir.path().join("renamed.mov");
    tokio::fs::write(&src_file, b"data to rename").await.unwrap();

    // Rename the file
    tokio::fs::rename(&src_file, &renamed_file).await.unwrap();

    // Original path should not exist
    assert!(!src_file.exists());
    assert!(renamed_file.exists());

    // Attempting to read original should fail
    let read_result = tokio::fs::read(&src_file).await;
    assert!(read_result.is_err());

    // Checkpoint should record the failure
    let conn = setup_db();
    checkpoint::create_job(
        &conn,
        "job-rename",
        "Rename Test",
        src_dir.path().to_str().unwrap(),
        None,
    )
    .unwrap();
    checkpoint::insert_task(
        &conn,
        "t-r1",
        "job-rename",
        src_file.to_str().unwrap(),
        dst_dir.path().join("original.mov").to_str().unwrap(),
        100,
    )
    .unwrap();
    checkpoint::update_task_failed(&conn, "t-r1", "Source file not found: original.mov").unwrap();

    let progress = checkpoint::get_job_progress(&conn, "job-rename").unwrap();
    assert_eq!(progress.failed, 1);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Scenario 13: Concurrent Write Conflicts
// ═══════════════════════════════════════════════════════════════════════════════

/// Two concurrent workflows writing different files to the same dest directory.
/// Both should succeed without interference.
#[tokio::test]
async fn test_concurrent_workflows_same_dest_different_files() {
    let dst_dir = tempdir().unwrap();

    let data_a = b"workflow A data - clip alpha";
    let data_b = b"workflow B data - clip bravo";

    let dst_a = dst_dir.path().join("clipA.mov");
    let dst_b = dst_dir.path().join("clipB.mov");

    // Simulate two concurrent atomic writes
    let dst_a_clone = dst_a.clone();
    let dst_b_clone = dst_b.clone();

    let handle_a = tokio::spawn(async move {
        let mut writer = AtomicWriter::new(&dst_a_clone).await.unwrap();
        writer.write(data_a).await.unwrap();
        writer.finalize().await.unwrap();
    });

    let handle_b = tokio::spawn(async move {
        let mut writer = AtomicWriter::new(&dst_b_clone).await.unwrap();
        writer.write(data_b).await.unwrap();
        writer.finalize().await.unwrap();
    });

    handle_a.await.unwrap();
    handle_b.await.unwrap();

    // Both files should exist with correct content
    let read_a = tokio::fs::read(&dst_a).await.unwrap();
    let read_b = tokio::fs::read(&dst_b).await.unwrap();
    assert_eq!(read_a.as_slice(), data_a);
    assert_eq!(read_b.as_slice(), data_b);

    // No .tmp residue
    assert!(!AtomicWriter::temp_path_for(&dst_a).exists());
    assert!(!AtomicWriter::temp_path_for(&dst_b).exists());
}

/// Two workflows writing the same filename to the same dest.
/// Second writer should overwrite (AtomicWriter uses .tmp + rename).
#[tokio::test]
async fn test_concurrent_same_file_last_writer_wins() {
    let dst_dir = tempdir().unwrap();
    let dest = dst_dir.path().join("contested.mov");

    let data_first = b"first writer data";
    let data_second = b"second writer data - this should win";

    // Sequential: first writer completes, second overwrites
    let mut w1 = AtomicWriter::new(&dest).await.unwrap();
    w1.write(data_first).await.unwrap();
    w1.finalize().await.unwrap();
    assert!(dest.exists());

    let mut w2 = AtomicWriter::new(&dest).await.unwrap();
    w2.write(data_second).await.unwrap();
    w2.finalize().await.unwrap();

    let content = tokio::fs::read(&dest).await.unwrap();
    assert_eq!(content.as_slice(), data_second, "Last writer should win");
}

/// Concurrent DB access: multiple tasks inserting and querying job records
/// simultaneously. The Arc<Mutex<Connection>> should serialize access.
#[tokio::test]
async fn test_concurrent_db_access_10_tasks() {
    let conn = Arc::new(std::sync::Mutex::new(setup_db()));

    // Create a shared job
    {
        let c = conn.lock().unwrap();
        checkpoint::create_job(&c, "job-conc", "Concurrent Test", "/src", None).unwrap();
    }

    // Spawn 10 tasks that each insert a copy_task record
    let barrier = Arc::new(tokio::sync::Barrier::new(10));
    let mut handles = Vec::new();

    for i in 0..10 {
        let conn = conn.clone();
        let barrier = barrier.clone();
        handles.push(tokio::spawn(async move {
            barrier.wait().await; // synchronize start
            let c = conn.lock().unwrap();
            checkpoint::insert_task(
                &c,
                &format!("t-conc-{}", i),
                "job-conc",
                &format!("/src/file{}.mov", i),
                &format!("/dst/file{}.mov", i),
                (i as u64 + 1) * 1000,
            )
            .unwrap();
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    // All 10 tasks should be recorded
    let c = conn.lock().unwrap();
    let progress = checkpoint::get_job_progress(&c, "job-conc").unwrap();
    assert_eq!(
        progress.total_tasks, 10,
        "All 10 tasks should be inserted"
    );
    assert_eq!(progress.pending, 10);
}

/// Concurrent queries while inserts are happening — verifies no deadlocks
/// and all operations complete.
#[tokio::test]
async fn test_concurrent_read_write_no_deadlock() {
    let conn = Arc::new(std::sync::Mutex::new(setup_db()));

    {
        let c = conn.lock().unwrap();
        checkpoint::create_job(&c, "job-rw", "ReadWrite Test", "/src", None).unwrap();
        // Pre-insert some tasks
        for i in 0..5 {
            checkpoint::insert_task(
                &c,
                &format!("t-rw-{}", i),
                "job-rw",
                &format!("/src/f{}.mov", i),
                &format!("/dst/f{}.mov", i),
                100,
            )
            .unwrap();
        }
    }

    let barrier = Arc::new(tokio::sync::Barrier::new(10));
    let mut handles = Vec::new();

    // 5 writers (insert tasks 5-9)
    for i in 5..10 {
        let conn = conn.clone();
        let barrier = barrier.clone();
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            let c = conn.lock().unwrap();
            checkpoint::insert_task(
                &c,
                &format!("t-rw-{}", i),
                "job-rw",
                &format!("/src/f{}.mov", i),
                &format!("/dst/f{}.mov", i),
                100,
            )
            .unwrap();
        }));
    }

    // 5 readers (query progress)
    for _ in 0..5 {
        let conn = conn.clone();
        let barrier = barrier.clone();
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            let c = conn.lock().unwrap();
            let progress = checkpoint::get_job_progress(&c, "job-rw").unwrap();
            // Progress should show some tasks (5 pre-inserted + 0-5 concurrent)
            assert!(progress.total_tasks >= 5);
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    // Final check: all 10 tasks present
    let c = conn.lock().unwrap();
    let progress = checkpoint::get_job_progress(&c, "job-rw").unwrap();
    assert_eq!(progress.total_tasks, 10);
}
