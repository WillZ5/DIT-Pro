//! Command-Level Integration Tests — Backend logic tests.
//!
//! Tests the library functions that back Tauri commands:
//! preset CRUD, settings persistence, DB init, error log,
//! report generation, volume info, hash correctness, MHL chain,
//! and debug bundle export.
//!
//! These tests do NOT require the Tauri runtime — they call the
//! underlying `app_lib::*` functions directly.

use app_lib::checkpoint;
use app_lib::config::{self, AppSettings};
use app_lib::copy_engine::atomic_writer::AtomicWriter;
use app_lib::db;
use app_lib::debug_bundle;
use app_lib::error_log::{self, ErrorLogFilter};
use app_lib::hash_engine::{hash_bytes, hash_file_sync, HashAlgorithm, HashEngineConfig};
use app_lib::mhl::{self, MhlConfig, MhlProcessType};
use app_lib::preset::{self, WorkflowPreset};
use app_lib::report;
use app_lib::volume;

use chrono::Utc;
use rusqlite::Connection;
use std::collections::HashMap;
use tempfile::tempdir;

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// In-memory DB with full schema (matches the existing test pattern).
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
        );
        CREATE TABLE IF NOT EXISTS error_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL DEFAULT (datetime('now')),
            error_code TEXT NOT NULL,
            severity TEXT NOT NULL,
            category TEXT NOT NULL,
            module TEXT NOT NULL,
            message TEXT NOT NULL,
            context_json TEXT,
            job_id TEXT,
            resolved INTEGER NOT NULL DEFAULT 0,
            resolved_at TEXT,
            app_version TEXT
        );",
    )
    .unwrap();
    conn
}

/// Create a minimal WorkflowPreset for testing.
fn make_test_preset(name: &str) -> WorkflowPreset {
    WorkflowPreset {
        id: String::new(), // will be assigned by create_preset
        name: name.to_string(),
        description: "Test preset".to_string(),
        hash_algorithms: vec!["XXH64".to_string(), "SHA256".to_string()],
        source_verify: true,
        post_verify: true,
        generate_mhl: true,
        buffer_size: 4_194_304,
        max_retries: 3,
        cascade: false,
        default_dest_paths: vec![],
        created_at: String::new(),
        updated_at: String::new(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 1. Preset CRUD
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn preset_crud_full_cycle() {
    let dir = tempdir().unwrap();
    let app_dir = dir.path();

    // Initial state: only built-in presets
    let store_before = preset::load_presets(app_dir).unwrap();
    let initial_count = store_before.presets.len();

    // CREATE
    let created = preset::create_preset(app_dir, make_test_preset("ARRI Daily")).unwrap();
    assert!(!created.id.is_empty(), "id must be assigned");
    assert_eq!(created.name, "ARRI Daily");
    assert!(!created.created_at.is_empty());

    let store = preset::load_presets(app_dir).unwrap();
    assert_eq!(store.presets.len(), initial_count + 1);

    // UPDATE
    let mut updated = created.clone();
    updated.name = "ARRI Daily V2".to_string();
    updated.cascade = true;
    let result = preset::update_preset(app_dir, updated).unwrap();
    assert_eq!(result.name, "ARRI Daily V2");
    assert!(result.cascade);

    // Verify update persisted
    let store = preset::load_presets(app_dir).unwrap();
    let found = store.presets.iter().find(|p| p.id == created.id).unwrap();
    assert_eq!(found.name, "ARRI Daily V2");

    // DELETE
    preset::delete_preset(app_dir, &created.id).unwrap();
    let store = preset::load_presets(app_dir).unwrap();
    assert_eq!(store.presets.len(), initial_count);
    assert!(store.presets.iter().all(|p| p.id != created.id));
}

#[test]
fn preset_create_multiple_unique_ids() {
    let dir = tempdir().unwrap();
    let p1 = preset::create_preset(dir.path(), make_test_preset("Preset A")).unwrap();
    let p2 = preset::create_preset(dir.path(), make_test_preset("Preset B")).unwrap();
    let p3 = preset::create_preset(dir.path(), make_test_preset("Preset C")).unwrap();
    assert_ne!(p1.id, p2.id);
    assert_ne!(p2.id, p3.id);
    assert_ne!(p1.id, p3.id);
}

// ═══════════════════════════════════════════════════════════════════════════════
// 2. Settings Save / Load
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn settings_save_and_reload_persists() {
    let dir = tempdir().unwrap();

    // Save custom settings
    let mut settings = AppSettings::default();
    settings.offload.buffer_size = 8_388_608;
    settings.offload.cascade = true;
    settings.offload.max_retries = 5;
    config::save_settings(dir.path(), &settings).unwrap();

    // Reload and verify
    let loaded = config::load_settings(dir.path()).unwrap();
    assert_eq!(loaded.offload.buffer_size, 8_388_608);
    assert!(loaded.offload.cascade);
    assert_eq!(loaded.offload.max_retries, 5);
}

#[test]
fn settings_default_values_sensible() {
    let dir = tempdir().unwrap();
    let settings = config::load_settings(dir.path()).unwrap();

    // Sensible defaults
    assert!(settings.offload.source_verify);
    assert!(settings.offload.post_verify);
    assert!(settings.offload.generate_mhl);
    assert!(settings.offload.buffer_size > 0);
    assert!(settings.offload.max_retries >= 1);
}

// ═══════════════════════════════════════════════════════════════════════════════
// 3. Database Init Idempotent
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn db_init_twice_no_error() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let path_str = db_path.to_str().unwrap();

    let conn1 = db::init_database(path_str).unwrap();
    drop(conn1);

    // Second init on same file should succeed (idempotent)
    let conn2 = db::init_database(path_str).unwrap();
    drop(conn2);
}

#[test]
fn db_init_in_memory() {
    let conn = db::init_database(":memory:").unwrap();
    // Should be able to query tables
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(count >= 4, "Expected at least 4 tables, got {}", count);
}

// ═══════════════════════════════════════════════════════════════════════════════
// 4. Error Log CRUD
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn error_log_write_read_resolve_clear() {
    let conn = setup_db();

    // Write
    let id1 = error_log::log_raw_error(
        &conn,
        "E1001",
        "error",
        "IO",
        "copy_engine",
        "Read failed on sector 42",
        None,
    )
    .unwrap();
    let id2 = error_log::log_raw_error(
        &conn,
        "E1002",
        "warning",
        "IO",
        "copy_engine",
        "Slow write detected",
        Some("job-123"),
    )
    .unwrap();
    assert!(id1 > 0);
    assert!(id2 > 0);

    // Read all
    let filter = ErrorLogFilter {
        severity: None,
        category: None,
        job_id: None,
        resolved: None,
        limit: None,
        offset: None,
    };
    let entries = error_log::query_error_log(&conn, &filter).unwrap();
    assert_eq!(entries.len(), 2);

    // Summary
    let summary = error_log::error_log_summary(&conn).unwrap();
    assert_eq!(summary.total, 2);
    assert_eq!(summary.error, 1);
    assert_eq!(summary.warning, 1);
    assert_eq!(summary.unresolved, 2);

    // Resolve
    let resolved = error_log::resolve_error(&conn, id1).unwrap();
    assert!(resolved);
    let summary = error_log::error_log_summary(&conn).unwrap();
    assert_eq!(summary.unresolved, 1);

    // Clear
    let cleared = error_log::clear_error_log(&conn, None).unwrap();
    assert_eq!(cleared, 2);
    let summary = error_log::error_log_summary(&conn).unwrap();
    assert_eq!(summary.total, 0);
}

#[test]
fn error_log_filter_by_severity() {
    let conn = setup_db();

    error_log::log_raw_error(&conn, "E1001", "critical", "IO", "mod", "msg1", None).unwrap();
    error_log::log_raw_error(&conn, "E1002", "error", "IO", "mod", "msg2", None).unwrap();
    error_log::log_raw_error(&conn, "E1003", "warning", "IO", "mod", "msg3", None).unwrap();

    let filter = ErrorLogFilter {
        severity: Some("critical".to_string()),
        category: None,
        job_id: None,
        resolved: None,
        limit: None,
        offset: None,
    };
    let entries = error_log::query_error_log(&conn, &filter).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].error_code, "E1001");
}

// ═══════════════════════════════════════════════════════════════════════════════
// 5. Report Generation
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn report_day_report_structure() {
    let conn = setup_db();
    let today = Utc::now().format("%Y-%m-%d").to_string();

    // Create a job with some tasks
    checkpoint::create_job(&conn, "job-rpt", "Report Test", "/src", None).unwrap();
    checkpoint::insert_task(&conn, "t-1", "job-rpt", "/src/a.mov", "/dst/a.mov", 1000).unwrap();
    checkpoint::insert_task(&conn, "t-2", "job-rpt", "/src/b.mov", "/dst/b.mov", 2000).unwrap();

    let day_report = report::get_day_report(&conn, &today).unwrap();
    assert_eq!(day_report.date, today);
    // Structure should have job summary
    assert!(day_report.total_jobs >= 1);
}

#[test]
fn report_job_report_structure() {
    let conn = setup_db();

    checkpoint::create_job(&conn, "job-jr", "Job Report Test", "/src", None).unwrap();
    checkpoint::insert_task(&conn, "t-1", "job-jr", "/src/a.mov", "/dst/a.mov", 5000).unwrap();

    let job_report = report::get_job_report(&conn, "job-jr").unwrap();
    assert_eq!(job_report.summary.job_id, "job-jr");
    assert_eq!(job_report.summary.job_name, "Job Report Test");
}

#[test]
fn report_dates_lists_job_dates() {
    let conn = setup_db();

    checkpoint::create_job(&conn, "job-d1", "Date Test", "/src", None).unwrap();
    let dates = report::get_report_dates(&conn).unwrap();
    assert!(!dates.is_empty(), "Should have at least one date");
}

#[test]
fn report_html_export_nonempty() {
    let conn = setup_db();
    let today = Utc::now().format("%Y-%m-%d").to_string();

    checkpoint::create_job(&conn, "job-html", "HTML Test", "/src", None).unwrap();
    checkpoint::insert_task(&conn, "t-1", "job-html", "/src/a.mov", "/dst/a.mov", 100).unwrap();

    let day_report = report::get_day_report(&conn, &today).unwrap();
    let html = report::render_day_report_html(&day_report);
    assert!(!html.is_empty());
    assert!(html.contains("<html") || html.contains("<!DOCTYPE"));

    // Save to file
    let dir = tempdir().unwrap();
    let path = report::save_report(dir.path(), "test-report.html", &html).unwrap();
    assert!(path.exists());
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(!content.is_empty());
}

// ═══════════════════════════════════════════════════════════════════════════════
// 6. Volume Detection
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn volume_list_has_at_least_one() {
    let volumes = volume::list_mounted_volumes().await.unwrap();
    assert!(
        !volumes.is_empty(),
        "Should detect at least one volume (system disk)"
    );

    // Each volume should have valid data
    for v in &volumes {
        assert!(!v.name.is_empty() || !v.mount_point.is_empty());
        assert!(v.total_bytes > 0, "Volume {} has 0 total bytes", v.name);
    }
}

#[test]
fn volume_space_for_root() {
    #[cfg(unix)]
    let root = std::path::Path::new("/");
    #[cfg(windows)]
    let root = std::path::Path::new("C:\\");

    let space = volume::get_volume_space(root).unwrap();
    assert!(space.total_bytes > 0);
    assert!(space.available_bytes > 0);
    assert!(space.available_bytes <= space.total_bytes);
    assert!(space.usage_percent >= 0.0 && space.usage_percent <= 100.0);
}

// ═══════════════════════════════════════════════════════════════════════════════
// 7. Debug Bundle
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn debug_bundle_generates_json() {
    let dir = tempdir().unwrap();
    let conn = setup_db();
    let settings = AppSettings::default();

    let path = debug_bundle::create_debug_bundle(&conn, dir.path(), &settings).unwrap();
    assert!(path.exists(), "Debug bundle file should exist");

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(!content.is_empty());

    // Should be valid JSON with system info (keys are camelCase)
    let json: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert!(
        json.get("systemInfo").is_some(),
        "Expected 'systemInfo' key in debug bundle JSON. Keys: {:?}",
        json.as_object().map(|o| o.keys().collect::<Vec<_>>())
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// 8. Hash Engine
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn hash_bytes_known_values() {
    let data = b"hello world";

    // XXH64
    let results = hash_bytes(data, &[HashAlgorithm::XXH64]);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].algorithm, HashAlgorithm::XXH64);
    assert!(!results[0].hex_digest.is_empty());

    // SHA256 known value
    let sha_results = hash_bytes(data, &[HashAlgorithm::SHA256]);
    assert_eq!(
        sha_results[0].hex_digest,
        "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
    );
}

#[test]
fn hash_bytes_multi_algorithm() {
    let data = b"test data for multi-hash";
    let results = hash_bytes(
        data,
        &[
            HashAlgorithm::XXH64,
            HashAlgorithm::SHA256,
            HashAlgorithm::MD5,
        ],
    );
    assert_eq!(results.len(), 3);
    // All should have non-empty hex digests
    for r in &results {
        assert!(!r.hex_digest.is_empty());
    }
    // Each should be unique
    let digests: Vec<&str> = results.iter().map(|r| r.hex_digest.as_str()).collect();
    assert_ne!(digests[0], digests[1]);
}

#[test]
fn hash_file_sync_correctness() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test.bin");
    let data = b"known file content for hash verification";
    std::fs::write(&file_path, data).unwrap();

    let config = HashEngineConfig {
        algorithms: vec![HashAlgorithm::SHA256],
        buffer_size: 4096,
    };
    let results = hash_file_sync(&file_path, &config).unwrap();
    assert_eq!(results.len(), 1);

    // Cross-verify with hash_bytes
    let direct = hash_bytes(data, &[HashAlgorithm::SHA256]);
    assert_eq!(results[0].hex_digest, direct[0].hex_digest);
}

// ═══════════════════════════════════════════════════════════════════════════════
// 9. MHL Create + Verify Chain
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn mhl_create_and_verify_chain() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("MHLTest");
    tokio::fs::create_dir_all(&root).await.unwrap();

    // Create a real file
    let file_data = b"camera footage data for MHL test";
    tokio::fs::write(root.join("A001C001.mov"), file_data)
        .await
        .unwrap();

    let mut history = mhl::load_or_create_history(&root).await.unwrap();

    let file_hash = hash_bytes(file_data, &[HashAlgorithm::XXH64]);
    let mut file_hashes = HashMap::new();
    file_hashes.insert("A001C001.mov".to_string(), file_hash);
    let mut file_metadata = HashMap::new();
    file_metadata.insert(
        "A001C001.mov".to_string(),
        (file_data.len() as u64, Utc::now()),
    );

    let manifest_path = mhl::create_generation(
        &mut history,
        &file_hashes,
        &file_metadata,
        MhlProcessType::Transfer,
        &MhlConfig::default(),
    )
    .await
    .unwrap();

    // Manifest file should exist
    assert!(manifest_path.exists());

    // Chain verification should pass
    let results = mhl::verify_chain(&history).await.unwrap();
    assert!(!results.is_empty());
    assert!(results[0].1, "Chain should be valid after creation");
}

#[tokio::test]
async fn mhl_multiple_generations() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("MultiGen");
    tokio::fs::create_dir_all(&root).await.unwrap();

    let mut history = mhl::load_or_create_history(&root).await.unwrap();

    // Gen 1
    let data1 = b"file1 data";
    tokio::fs::write(root.join("clip1.mov"), data1)
        .await
        .unwrap();
    let mut hashes1 = HashMap::new();
    hashes1.insert(
        "clip1.mov".to_string(),
        hash_bytes(data1, &[HashAlgorithm::XXH64]),
    );
    let mut meta1 = HashMap::new();
    meta1.insert("clip1.mov".to_string(), (data1.len() as u64, Utc::now()));
    mhl::create_generation(
        &mut history,
        &hashes1,
        &meta1,
        MhlProcessType::Transfer,
        &MhlConfig::default(),
    )
    .await
    .unwrap();

    // Gen 2
    let data2 = b"file2 data";
    tokio::fs::write(root.join("clip2.mov"), data2)
        .await
        .unwrap();
    let mut hashes2 = HashMap::new();
    hashes2.insert(
        "clip2.mov".to_string(),
        hash_bytes(data2, &[HashAlgorithm::XXH64]),
    );
    let mut meta2 = HashMap::new();
    meta2.insert("clip2.mov".to_string(), (data2.len() as u64, Utc::now()));
    mhl::create_generation(
        &mut history,
        &hashes2,
        &meta2,
        MhlProcessType::Transfer,
        &MhlConfig::default(),
    )
    .await
    .unwrap();

    // Both generations should verify
    let results = mhl::verify_chain(&history).await.unwrap();
    assert_eq!(results.len(), 2);
    assert!(results[0].1, "Gen 1 should verify");
    assert!(results[1].1, "Gen 2 should verify");
}

// ═══════════════════════════════════════════════════════════════════════════════
// 10. Small File Offload (End-to-End through Checkpoint)
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn offload_small_files_checkpoint_round_trip() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();

    // Create 5 small source files
    let file_contents: Vec<Vec<u8>> = (0..5)
        .map(|i| format!("file {} content with some unique data {}", i, i * 37).into_bytes())
        .collect();

    for (i, content) in file_contents.iter().enumerate() {
        let name = format!("A001C{:04}.mov", i + 1);
        tokio::fs::write(src_dir.path().join(&name), content)
            .await
            .unwrap();
    }

    let conn = setup_db();
    checkpoint::create_job(
        &conn,
        "job-e2e",
        "E2E Test",
        src_dir.path().to_str().unwrap(),
        None,
    )
    .unwrap();

    // Insert tasks
    for (i, content) in file_contents.iter().enumerate() {
        let name = format!("A001C{:04}.mov", i + 1);
        checkpoint::insert_task(
            &conn,
            &format!("t-{}", i),
            "job-e2e",
            src_dir.path().join(&name).to_str().unwrap(),
            dst_dir.path().join(&name).to_str().unwrap(),
            content.len() as u64,
        )
        .unwrap();
    }

    // Simulate copy: use AtomicWriter for each file
    for (i, content) in file_contents.iter().enumerate() {
        let name = format!("A001C{:04}.mov", i + 1);
        let dest = dst_dir.path().join(&name);

        let mut writer = AtomicWriter::new(&dest).await.unwrap();
        writer.write(content).await.unwrap();
        writer.finalize().await.unwrap();

        // Hash and mark complete
        let hashes = hash_bytes(content, &[HashAlgorithm::XXH64, HashAlgorithm::SHA256]);
        let task_hashes = checkpoint::TaskHashes {
            xxh64: hashes
                .iter()
                .find(|h| h.algorithm == HashAlgorithm::XXH64)
                .map(|h| h.hex_digest.clone()),
            sha256: hashes
                .iter()
                .find(|h| h.algorithm == HashAlgorithm::SHA256)
                .map(|h| h.hex_digest.clone()),
            ..Default::default()
        };
        checkpoint::update_task_completed(&conn, &format!("t-{}", i), &task_hashes).unwrap();
    }

    // Verify progress
    let progress = checkpoint::get_job_progress(&conn, "job-e2e").unwrap();
    assert_eq!(progress.completed, 5);
    assert_eq!(progress.pending, 0);
    assert_eq!(progress.failed, 0);

    // Verify file content and hashes match source
    for (i, content) in file_contents.iter().enumerate() {
        let name = format!("A001C{:04}.mov", i + 1);
        let dest = dst_dir.path().join(&name);
        let read_back = tokio::fs::read(&dest).await.unwrap();
        assert_eq!(&read_back, content, "File {} content mismatch", name);

        // No .tmp residue
        assert!(!AtomicWriter::temp_path_for(&dest).exists());
    }

    // Recovery should return empty (all done)
    let recovered = checkpoint::recover_job(&conn, "job-e2e").await.unwrap();
    assert!(recovered.is_empty());
}

// ═══════════════════════════════════════════════════════════════════════════════
// 11. Checkpoint Job Lifecycle
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn checkpoint_job_create_and_progress() {
    let conn = setup_db();

    checkpoint::create_job(&conn, "job-lc", "Lifecycle Test", "/src", None).unwrap();
    checkpoint::insert_task(&conn, "t-1", "job-lc", "/src/a.mov", "/dst/a.mov", 100).unwrap();
    checkpoint::insert_task(&conn, "t-2", "job-lc", "/src/b.mov", "/dst/b.mov", 200).unwrap();
    checkpoint::insert_task(&conn, "t-3", "job-lc", "/src/c.mov", "/dst/c.mov", 300).unwrap();

    let progress = checkpoint::get_job_progress(&conn, "job-lc").unwrap();
    assert_eq!(progress.total_tasks, 3);
    assert_eq!(progress.pending, 3);
    assert_eq!(progress.completed, 0);
    assert_eq!(progress.failed, 0);
}
