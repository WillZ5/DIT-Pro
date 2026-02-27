//! End-to-End Stress Tests — Validates MVP acceptance criteria under load.
//!
//! These tests exercise the full offload workflow with real file I/O at scale:
//! - AC-1: Large file count/size → zero loss, zero mismatch
//! - AC-2: Interrupt and resume → no dirty data
//! - AC-4: Copy throughput benchmark
//! - AC-6: Cascading copy at scale
//!
//! Run with:
//!   cargo test --release --test stress_tests -- --nocapture --test-threads=1

use app_lib::checkpoint;
use app_lib::db;
use app_lib::hash_engine::{self, HashAlgorithm, HashEngineConfig};
use app_lib::mhl;
use app_lib::workflow::{CancelToken, OffloadConfig, OffloadEvent, OffloadPhase, OffloadWorkflow};
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::tempdir;
use tokio::sync::mpsc;

// ─── Helper Functions ────────────────────────────────────────────────────────

/// Create a file-based SQLite database with production schema (tests WAL under load).
fn setup_real_db(db_path: &Path) -> Connection {
    let conn = db::init_database(db_path.to_str().unwrap()).unwrap();
    conn
}

/// Wrap a Connection in Arc<Mutex<>> for OffloadWorkflow.
fn wrap_db(conn: Connection) -> Arc<std::sync::Mutex<Connection>> {
    Arc::new(std::sync::Mutex::new(conn))
}

/// Generate a file of specified size using a deterministic seed.
/// Writes in 1MB chunks to avoid OOM for large files.
fn generate_test_file(path: &Path, size: usize, seed: u64) {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut f = std::fs::File::create(path).unwrap();
    let chunk_size = 1024 * 1024; // 1MB
    let mut written = 0usize;
    let mut buf = vec![0u8; chunk_size.min(size)];
    while written < size {
        let remaining = size - written;
        let to_write = remaining.min(chunk_size);
        for (i, byte) in buf[..to_write].iter_mut().enumerate() {
            let pos = (written + i) as u64;
            *byte = (pos.wrapping_mul(seed.wrapping_add(7))
                ^ pos.wrapping_mul(seed.wrapping_mul(13).wrapping_add(37)))
                as u8;
        }
        f.write_all(&buf[..to_write]).unwrap();
        written += to_write;
    }
    f.sync_all().unwrap();
}

/// Generate N files of varying sizes in a directory. Returns (rel_path, file_size) pairs.
/// Size distribution simulates real production footage:
///   10% @ 10MB, 10% @ 5MB, 10% @ 1MB, 20% @ 500KB,
///   20% @ 100KB, 20% @ 10KB, 10% @ 1KB
fn generate_test_files_varied(dir: &Path, count: usize, seed: u64) -> Vec<(String, u64)> {
    let mut files = Vec::with_capacity(count);
    for i in 0..count {
        let size: usize = match i % 10 {
            0 => 10 * 1024 * 1024, // 10MB
            1 => 5 * 1024 * 1024,  // 5MB
            2 => 1024 * 1024,      // 1MB
            3 | 4 => 500 * 1024,   // 500KB
            5 | 6 => 100 * 1024,   // 100KB
            7 | 8 => 10 * 1024,    // 10KB
            _ => 1024,             // 1KB
        };

        // Distribute files across subdirectories for realism
        let subdir = match i % 5 {
            0 => "A001_C001",
            1 => "A001_C002",
            2 => "B001_C001",
            3 => "audio",
            _ => "metadata",
        };
        let ext = if i % 3 == 0 {
            "mov"
        } else if i % 3 == 1 {
            "wav"
        } else {
            "mxf"
        };
        let rel = format!("{}/clip_{:05}.{}", subdir, i, ext);

        generate_test_file(&dir.join(&rel), size, seed.wrapping_add(i as u64));
        files.push((rel, size as u64));
    }
    files
}

/// Build an OffloadConfig with common defaults.
fn make_config(
    job_id: &str,
    source: &Path,
    dests: &[PathBuf],
    source_verify: bool,
    post_verify: bool,
    generate_mhl: bool,
    cascade: bool,
) -> OffloadConfig {
    OffloadConfig {
        job_id: job_id.into(),
        job_name: format!("Stress: {}", job_id),
        source_path: source.to_path_buf(),
        dest_paths: dests.to_vec(),
        hash_algorithms: vec![HashAlgorithm::XXH64, HashAlgorithm::SHA256],
        buffer_size: 4 * 1024 * 1024,
        source_verify,
        post_verify,
        generate_mhl,
        max_retries: 3,
        cascade,
        conflict_resolutions: HashMap::new(),
    }
}

/// Deep-verify that every file in source exists in all destinations with matching SHA256.
async fn verify_all_files_match(source: &Path, dests: &[PathBuf]) -> bool {
    let hash_cfg = HashEngineConfig {
        algorithms: vec![HashAlgorithm::SHA256],
        buffer_size: 4 * 1024 * 1024,
    };

    fn walk_files(dir: &Path, prefix: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                // Skip MHL metadata directory
                if path.file_name().map(|n| n == "ascmhl").unwrap_or(false) {
                    continue;
                }
                walk_files(&path, prefix, out);
            } else {
                let rel = path
                    .strip_prefix(prefix)
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                // Skip .DS_Store and hidden files
                if !rel.starts_with('.') {
                    out.push(rel);
                }
            }
        }
    }

    let mut src_files = Vec::new();
    walk_files(source, source, &mut src_files);
    src_files.sort();

    for rel in &src_files {
        let src_hash = hash_engine::hash_file(&source.join(rel), &hash_cfg)
            .await
            .unwrap();
        let src_digest = &src_hash[0].hex_digest;

        for dest in dests {
            let dest_file = dest.join(rel);
            if !dest_file.exists() {
                eprintln!("MISSING: {} not found in {:?}", rel, dest);
                return false;
            }
            let dest_hash = hash_engine::hash_file(&dest_file, &hash_cfg).await.unwrap();
            if dest_hash[0].hex_digest != *src_digest {
                eprintln!(
                    "MISMATCH: {} — src={} dest={}",
                    rel, src_digest, dest_hash[0].hex_digest
                );
                return false;
            }
        }
    }
    true
}

/// Collect all events from the receiver until it closes.
async fn drain_events(mut rx: mpsc::UnboundedReceiver<OffloadEvent>) -> Vec<OffloadEvent> {
    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        events.push(ev);
    }
    events
}

// ─── Category 1: Large File Count ────────────────────────────────────────────

#[tokio::test]
async fn stress_1000_files_2_dests() {
    let tmp = tempdir().unwrap();
    let source = tmp.path().join("source");
    let dest1 = tmp.path().join("dest1");
    let dest2 = tmp.path().join("dest2");
    std::fs::create_dir_all(&source).unwrap();

    // Generate 1000 files (~1.72GB total)
    let files = generate_test_files_varied(&source, 1000, 42);
    let total_source_bytes: u64 = files.iter().map(|(_, s)| s).sum();
    assert!(total_source_bytes > 1_000_000_000, "Source should be >1GB");

    let db_path = tmp.path().join("stress.db");
    let conn = setup_real_db(&db_path);
    let db = wrap_db(conn);
    let (tx, rx) = mpsc::unbounded_channel();

    let config = make_config(
        "stress-1000",
        &source,
        &[dest1.clone(), dest2.clone()],
        true,
        true,
        true,
        false,
    );

    let workflow = OffloadWorkflow::new(config, db.clone(), tx);
    let result = workflow.execute().await.unwrap();

    // Core assertions
    assert!(result.success, "Workflow must succeed");
    assert_eq!(result.total_files, 1000);
    assert_eq!(result.failed_files, 0);
    assert_eq!(result.total_bytes, total_source_bytes);
    assert!(!result.mhl_paths.is_empty(), "MHL should be generated");

    // Verify every file hash matches
    assert!(
        verify_all_files_match(&source, &[dest1.clone(), dest2.clone()]).await,
        "All files must match source"
    );

    // Verify DB state
    {
        let conn = db.lock().unwrap();
        let progress = checkpoint::get_job_progress(&conn, "stress-1000").unwrap();
        assert_eq!(progress.completed, 2000); // 1000 files × 2 dests
        assert_eq!(progress.pending, 0);
        assert_eq!(progress.failed, 0);
    }

    // Verify MHL chain for both destinations
    for dest in [&dest1, &dest2] {
        let history = mhl::load_or_create_history(dest).await.unwrap();
        let chain_results = mhl::verify_chain(&history).await.unwrap();
        assert!(!chain_results.is_empty());
        assert!(
            chain_results.iter().all(|(_, valid)| *valid),
            "MHL chain must be valid"
        );
    }

    // Verify Complete event was emitted
    drop(workflow);
    let events = drain_events(rx).await;
    assert!(events
        .iter()
        .any(|e| matches!(e, OffloadEvent::Complete { .. })));
}

// ─── Category 2: Large File Size ─────────────────────────────────────────────

#[tokio::test]
async fn stress_large_files_500mb() {
    let tmp = tempdir().unwrap();
    let source = tmp.path().join("source");
    let dest1 = tmp.path().join("dest1");
    let dest2 = tmp.path().join("dest2");
    std::fs::create_dir_all(&source).unwrap();

    // 5 × 500MB = 2.5GB source
    let file_size = 500 * 1024 * 1024;
    for i in 0..5 {
        generate_test_file(
            &source.join(format!("large_clip_{:03}.mov", i)),
            file_size,
            (i + 100) as u64,
        );
    }

    let db_path = tmp.path().join("stress.db");
    let conn = setup_real_db(&db_path);
    let db = wrap_db(conn);
    let (tx, _rx) = mpsc::unbounded_channel();

    let config = make_config(
        "stress-large",
        &source,
        &[dest1.clone(), dest2.clone()],
        true,
        true,
        false,
        false,
    );

    let workflow = OffloadWorkflow::new(config, db.clone(), tx);
    let result = workflow.execute().await.unwrap();

    assert!(result.success);
    assert_eq!(result.total_files, 5);
    assert_eq!(result.total_bytes, 5 * file_size as u64);
    assert_eq!(result.failed_files, 0);

    // Verify exact byte count for each file in each destination
    for i in 0..5 {
        let name = format!("large_clip_{:03}.mov", i);
        let src_len = std::fs::metadata(source.join(&name)).unwrap().len();
        let d1_len = std::fs::metadata(dest1.join(&name)).unwrap().len();
        let d2_len = std::fs::metadata(dest2.join(&name)).unwrap().len();
        assert_eq!(src_len, d1_len, "dest1 size mismatch for {}", name);
        assert_eq!(src_len, d2_len, "dest2 size mismatch for {}", name);
    }

    // Full hash verification
    assert!(verify_all_files_match(&source, &[dest1, dest2]).await);
}

// ─── Category 3: Multi-Destination Concurrent ────────────────────────────────

#[tokio::test]
async fn stress_50_files_4_dests() {
    let tmp = tempdir().unwrap();
    let source = tmp.path().join("source");
    let dests: Vec<PathBuf> = (0..4)
        .map(|i| tmp.path().join(format!("dest_{}", i)))
        .collect();
    std::fs::create_dir_all(&source).unwrap();

    let _files = generate_test_files_varied(&source, 50, 77);

    let db_path = tmp.path().join("stress.db");
    let conn = setup_real_db(&db_path);
    let db = wrap_db(conn);
    let (tx, _rx) = mpsc::unbounded_channel();

    let config = make_config("stress-4dests", &source, &dests, true, true, true, false);

    let workflow = OffloadWorkflow::new(config, db.clone(), tx);
    let result = workflow.execute().await.unwrap();

    assert!(result.success);
    assert_eq!(result.total_files, 50);
    assert_eq!(result.failed_files, 0);

    // All 4 copies identical to source
    assert!(verify_all_files_match(&source, &dests).await);

    // DB: 50 files × 4 dests = 200 completed tasks
    {
        let conn = db.lock().unwrap();
        let progress = checkpoint::get_job_progress(&conn, "stress-4dests").unwrap();
        assert_eq!(progress.completed, 200);
        assert_eq!(progress.failed, 0);
    }

    // MHL generated for all 4 destinations
    for dest in &dests {
        assert!(
            dest.join("ascmhl").exists(),
            "MHL dir must exist for {:?}",
            dest
        );
    }
}

// ─── Category 4: Cascading Copy ─────────────────────────────────────────────

#[tokio::test]
async fn stress_cascade_100_files_3_dests() {
    let tmp = tempdir().unwrap();
    let source = tmp.path().join("source");
    let primary = tmp.path().join("primary");
    let secondary1 = tmp.path().join("secondary1");
    let secondary2 = tmp.path().join("secondary2");
    std::fs::create_dir_all(&source).unwrap();

    let _files = generate_test_files_varied(&source, 100, 55);

    let db_path = tmp.path().join("stress.db");
    let conn = setup_real_db(&db_path);
    let db = wrap_db(conn);
    let (tx, rx) = mpsc::unbounded_channel();

    let config = make_config(
        "stress-cascade",
        &source,
        &[primary.clone(), secondary1.clone(), secondary2.clone()],
        true,
        true,
        true,
        true, // cascade=true
    );

    let workflow = OffloadWorkflow::new(config, db.clone(), tx);
    let result = workflow.execute().await.unwrap();

    assert!(result.success);
    assert_eq!(result.total_files, 100);
    assert_eq!(result.failed_files, 0);

    // All 3 destinations match source
    assert!(
        verify_all_files_match(
            &source,
            &[primary.clone(), secondary1.clone(), secondary2.clone()]
        )
        .await
    );

    // Cascading phase event must have been emitted
    drop(workflow);
    let events = drain_events(rx).await;
    let has_cascade = events.iter().any(|e| {
        matches!(
            e,
            OffloadEvent::PhaseChanged {
                phase: OffloadPhase::Cascading,
                ..
            }
        )
    });
    assert!(has_cascade, "Cascading phase event must be emitted");

    // DB: 100 files × 3 dests = 300 tasks
    {
        let conn = db.lock().unwrap();
        let progress = checkpoint::get_job_progress(&conn, "stress-cascade").unwrap();
        assert_eq!(progress.completed, 300);
        assert_eq!(progress.failed, 0);
    }
}

// ─── Category 5: Interrupt & Resume ──────────────────────────────────────────

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn stress_interrupt_resume_200_files() {
    let tmp = tempdir().unwrap();
    let source = tmp.path().join("source");
    let dest = tmp.path().join("dest");
    std::fs::create_dir_all(&source).unwrap();

    // 200 × 100KB files = 20MB total (small for fast cancel timing)
    for i in 0..200 {
        generate_test_file(
            &source.join(format!("clip_{:04}.mov", i)),
            100 * 1024,
            i as u64,
        );
    }

    let db_path = tmp.path().join("stress.db");
    let conn = setup_real_db(&db_path);
    let db = wrap_db(conn);

    // ── Phase 1: Start and cancel after some progress ──
    let cancel = CancelToken::new();
    let cancel_clone = cancel.clone();
    let db_clone = db.clone();
    let source_clone = source.clone();
    let dest_clone = dest.clone();

    let (tx1, _rx1) = mpsc::unbounded_channel();

    let handle = tokio::spawn(async move {
        let config = OffloadConfig {
            job_id: "stress-interrupt".into(),
            job_name: "Interrupt Test".into(),
            source_path: source_clone,
            dest_paths: vec![dest_clone],
            hash_algorithms: vec![HashAlgorithm::XXH64],
            buffer_size: 4 * 1024 * 1024,
            source_verify: false, // skip for speed
            post_verify: false,
            generate_mhl: false,
            max_retries: 3,
            cascade: false,
            conflict_resolutions: HashMap::new(),
        };
        let workflow = OffloadWorkflow::with_cancel(config, db_clone, tx1, cancel_clone);
        workflow.execute().await
    });

    // Wait for ≥30 tasks completed, then cancel
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let conn = db.lock().unwrap();
        if let Ok(progress) = checkpoint::get_job_progress(&conn, "stress-interrupt") {
            if progress.completed >= 30 {
                cancel.cancel();
                break;
            }
        }
    }

    // Workflow should error on cancellation
    let result1 = handle.await.unwrap();
    assert!(result1.is_err(), "Cancelled workflow should return error");

    // Check partial progress
    let completed_before;
    {
        let conn = db.lock().unwrap();
        let progress = checkpoint::get_job_progress(&conn, "stress-interrupt").unwrap();
        completed_before = progress.completed;
        assert!(
            completed_before >= 30,
            "At least 30 tasks completed before cancel"
        );
        assert!(completed_before < 200, "Not all tasks should be completed");
    }

    // ── Phase 2: Recover and Resume ──
    {
        {
            let conn = db.lock().unwrap();
            let _recovered = checkpoint::recover_job(&conn, "stress-interrupt")
                .await
                .unwrap();
        }

        let conn = db.lock().unwrap();
        let progress = checkpoint::get_job_progress(&conn, "stress-interrupt").unwrap();
        assert_eq!(
            progress.copying, 0,
            "No tasks stuck in 'copying' after recovery"
        );
    }

    let (tx2, _rx2) = mpsc::unbounded_channel();
    let config2 = OffloadConfig {
        job_id: "stress-interrupt".into(),
        job_name: "Resume Test".into(),
        source_path: source.clone(),
        dest_paths: vec![dest.clone()],
        hash_algorithms: vec![HashAlgorithm::XXH64],
        buffer_size: 4 * 1024 * 1024,
        source_verify: false,
        post_verify: true, // verify everything on resume
        generate_mhl: false,
        max_retries: 3,
        cascade: false,
        conflict_resolutions: HashMap::new(),
    };

    let workflow2 = OffloadWorkflow::new(config2, db.clone(), tx2);
    let result2 = workflow2.execute_resume().await.unwrap();
    assert!(result2.success, "Resume must succeed");

    // ── Final verification ──

    // All 200 files must exist in dest
    for i in 0..200 {
        let name = format!("clip_{:04}.mov", i);
        assert!(
            dest.join(&name).exists(),
            "File {} must exist after resume",
            name
        );
    }

    // No .tmp files remain
    fn check_no_tmp(dir: &Path) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                check_no_tmp(&path);
            } else {
                assert!(
                    !path.to_string_lossy().ends_with(".tmp"),
                    "No .tmp files should remain: {:?}",
                    path
                );
            }
        }
    }
    check_no_tmp(&dest);

    // All hashes match source
    assert!(verify_all_files_match(&source, std::slice::from_ref(&dest)).await);

    // DB: all 200 tasks completed or skipped
    {
        let conn = db.lock().unwrap();
        let progress = checkpoint::get_job_progress(&conn, "stress-interrupt").unwrap();
        assert_eq!(progress.completed, 200);
        assert_eq!(progress.pending, 0);
        assert_eq!(progress.failed, 0);
    }
}

// ─── Category 6: Copy Throughput Benchmark ───────────────────────────────────

#[tokio::test]
async fn stress_throughput_benchmark() {
    use std::time::Instant;

    let tmp = tempdir().unwrap();
    let source_file = tmp.path().join("benchmark_source.bin");
    let dest_raw = tmp.path().join("dest_raw.bin");
    let dest_engine = tmp.path().join("dest_engine");
    std::fs::create_dir_all(&dest_engine).unwrap();

    let file_size: usize = 512 * 1024 * 1024; // 512MB (compromise: fast enough but meaningful)
    generate_test_file(&source_file, file_size, 12345);

    let raw_mbps: f64;
    let raw_secs: f64;

    // ── Baseline: userspace buffered copy (fair comparison) ──
    // NOTE: We don't use tokio::fs::copy because on macOS it uses kernel-level
    // fcopyfile/sendfile which is zero-copy and unfairly fast. Instead we do a
    // manual read→write loop in userspace to match what the DIT engine does.
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut src = tokio::fs::File::open(&source_file).await.unwrap();
        let mut dst = tokio::fs::File::create(&dest_raw).await.unwrap();
        let mut buf = vec![0u8; 4 * 1024 * 1024]; // 4MB buffer, same as DIT default
        let start_raw = Instant::now();
        loop {
            let n = src.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            dst.write_all(&buf[..n]).await.unwrap();
        }
        dst.flush().await.unwrap();
        let raw_duration = start_raw.elapsed();
        raw_mbps = (file_size as f64 / (1024.0 * 1024.0)) / raw_duration.as_secs_f64();
        raw_secs = raw_duration.as_secs_f64();
    }

    tokio::fs::remove_file(&dest_raw).await.unwrap();

    // ── DIT copy_engine with inline hashing (XXH64 + SHA256) ──
    // We use the full workflow for a single file to include all overhead
    let db_path = tmp.path().join("bench.db");
    let conn = setup_real_db(&db_path);
    let db = wrap_db(conn);
    let (tx, _rx) = mpsc::unbounded_channel();

    // Need to put source in a directory (workflow scans dirs)
    let source_dir = tmp.path().join("bench_source");
    std::fs::create_dir_all(&source_dir).unwrap();
    std::fs::rename(&source_file, source_dir.join("benchmark.bin")).unwrap();

    let config = make_config(
        "bench",
        &source_dir,
        std::slice::from_ref(&dest_engine),
        false,
        false,
        false,
        false, // no verify, no MHL — pure copy+hash
    );

    let start_engine = Instant::now();
    let workflow = OffloadWorkflow::new(config, db, tx);
    let result = workflow.execute().await.unwrap();
    let engine_duration = start_engine.elapsed();

    assert!(result.success);
    assert_eq!(result.total_files, 1);
    let engine_mbps = (file_size as f64 / (1024.0 * 1024.0)) / engine_duration.as_secs_f64();
    let engine_secs = engine_duration.as_secs_f64();

    let efficiency = engine_mbps / raw_mbps * 100.0;

    println!("\n╔══════════════════════════════════════════════════════╗");
    println!("║        DIT Pro Copy Throughput Benchmark          ║");
    println!("╠══════════════════════════════════════════════════════╣");
    println!(
        "║ File size:                 {:>6} MB                  ║",
        file_size / (1024 * 1024)
    );
    println!(
        "║ Raw buffered copy:    {:>8.1} MB/s ({:.2}s)          ║",
        raw_mbps, raw_secs
    );
    println!(
        "║ DIT (copy+XXH64+SHA256): {:>6.1} MB/s ({:.2}s)       ║",
        engine_mbps, engine_secs
    );
    println!(
        "║ Efficiency vs raw:       {:>5.1}%                     ║",
        efficiency
    );
    println!("║ Target: >= 30% (dual-hash adds significant overhead) ║");
    println!("╚══════════════════════════════════════════════════════╝\n");

    // Assert minimum absolute throughput — only meaningful in release builds.
    // Debug builds are 10-20x slower due to missing optimizations.
    if cfg!(not(debug_assertions)) {
        assert!(
            engine_mbps >= 50.0,
            "Copy engine must achieve >= 50 MB/s in release, got {:.1} MB/s",
            engine_mbps
        );
    } else {
        println!(
            "⚠ Skipping throughput assertion in debug build ({:.1} MB/s)",
            engine_mbps
        );
    }

    // Note: Relative efficiency vs raw will be low because the raw baseline reads
    // from OS file cache (>2GB/s) while DIT engine computes XXH64+SHA256 inline.
    // On real disk I/O, efficiency would be much higher. The absolute throughput
    // is what matters for production use.
}

// ─── Category 7: Concurrent Jobs ─────────────────────────────────────────────

#[tokio::test]
async fn stress_3_concurrent_jobs() {
    let tmp = tempdir().unwrap();

    // Create 3 independent source directories
    let mut sources = Vec::new();
    for i in 0..3 {
        let src = tmp.path().join(format!("source_{}", i));
        std::fs::create_dir_all(&src).unwrap();
        for j in 0..50 {
            generate_test_file(
                &src.join(format!("clip_{:04}.mov", j)),
                (50 + j * 10) * 1024, // 50KB-540KB
                (i * 1000 + j) as u64,
            );
        }
        sources.push(src);
    }

    let dests: Vec<PathBuf> = (0..3)
        .map(|i| tmp.path().join(format!("dest_{}", i)))
        .collect();

    // Shared database (tests WAL concurrent writes)
    let db_path = tmp.path().join("concurrent.db");
    let conn = setup_real_db(&db_path);
    let db = wrap_db(conn);

    // Launch 3 concurrent workflows
    let mut handles = Vec::new();
    for i in 0..3usize {
        let db_clone = db.clone();
        let source = sources[i].clone();
        let dest = dests[i].clone();

        handles.push(tokio::spawn(async move {
            let (tx, _rx) = mpsc::unbounded_channel();
            let config = OffloadConfig {
                job_id: format!("concurrent-{}", i),
                job_name: format!("Job {}", i),
                source_path: source,
                dest_paths: vec![dest],
                hash_algorithms: vec![HashAlgorithm::XXH64],
                buffer_size: 4 * 1024 * 1024,
                source_verify: true,
                post_verify: true,
                generate_mhl: false,
                max_retries: 3,
                cascade: false,
                conflict_resolutions: HashMap::new(),
            };
            let workflow = OffloadWorkflow::new(config, db_clone, tx);
            workflow.execute().await
        }));
    }

    // Wait for all 3 to complete
    for (i, handle) in handles.into_iter().enumerate() {
        let result = handle.await.unwrap().unwrap();
        assert!(result.success, "Job {} must succeed", i);
        assert_eq!(result.total_files, 50, "Job {} must copy 50 files", i);
        assert_eq!(result.failed_files, 0, "Job {} must have 0 failures", i);
    }

    // Verify no cross-contamination: each dest only has its own 50 files
    for (i, dest_dir) in dests.iter().enumerate() {
        let file_count = std::fs::read_dir(dest_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .count();
        assert_eq!(file_count, 50, "Dest {} must have exactly 50 files", i);
    }

    // Verify DB records are independent
    {
        let conn = db.lock().unwrap();
        for i in 0..3 {
            let progress =
                checkpoint::get_job_progress(&conn, &format!("concurrent-{}", i)).unwrap();
            assert_eq!(progress.completed, 50, "Job {} should have 50 completed", i);
            assert_eq!(progress.pending, 0);
            assert_eq!(progress.failed, 0);
        }
    }

    // Verify file correctness
    for i in 0..3 {
        assert!(
            verify_all_files_match(&sources[i], &[dests[i].clone()]).await,
            "Job {} files must match source",
            i
        );
    }
}

// ─── Category 8: MHL Chain Integrity Under Load ─────────────────────────────

#[tokio::test]
async fn stress_mhl_chain_100_files() {
    let tmp = tempdir().unwrap();
    let source = tmp.path().join("source");
    let dest = tmp.path().join("dest");
    std::fs::create_dir_all(&source).unwrap();

    let _files = generate_test_files_varied(&source, 100, 88);

    let db_path = tmp.path().join("stress.db");
    let conn = setup_real_db(&db_path);
    let db = wrap_db(conn);
    let (tx, _rx) = mpsc::unbounded_channel();

    let config = make_config(
        "stress-mhl",
        &source,
        std::slice::from_ref(&dest),
        true,
        true,
        true,
        false,
    );

    let workflow = OffloadWorkflow::new(config, db, tx);
    let result = workflow.execute().await.unwrap();

    assert!(result.success);
    assert_eq!(result.total_files, 100);
    assert_eq!(result.failed_files, 0);
    assert!(!result.mhl_paths.is_empty(), "MHL paths must be returned");

    // MHL directory exists
    let ascmhl_dir = dest.join("ascmhl");
    assert!(ascmhl_dir.exists(), "ascmhl directory must exist");
    assert!(
        ascmhl_dir.join("ascmhl_chain.xml").exists(),
        "chain file must exist"
    );

    // Load and verify chain
    let history = mhl::load_or_create_history(&dest).await.unwrap();
    assert_eq!(history.chain.len(), 1, "Should have 1 generation");
    assert_eq!(history.chain[0].sequence_nr, 1);

    // Chain integrity check
    let chain_results = mhl::verify_chain(&history).await.unwrap();
    assert_eq!(chain_results.len(), 1);
    assert!(chain_results[0].1, "Chain entry must be valid");

    // Manifest should contain hash entries for all 100 files
    let manifest_path = &result.mhl_paths[0];
    let manifest_content = tokio::fs::read_to_string(manifest_path).await.unwrap();
    assert!(
        manifest_content.contains("<hashlist"),
        "Manifest must be valid XML"
    );

    // Count <hash> entries — should be at least 100
    let hash_count = manifest_content.matches("<hash>").count();
    assert!(
        hash_count >= 100,
        "Manifest must contain >= 100 hash entries, got {}",
        hash_count
    );

    // ── Tamper detection test ──
    // Modify a byte in the manifest and verify chain detects it
    let tampered = manifest_content.replacen("<hash>", "<HASH>", 1);
    tokio::fs::write(manifest_path, &tampered).await.unwrap();

    // Re-verify with same history (reference hash unchanged)
    let tamper_results = mhl::verify_chain(&history).await.unwrap();
    assert!(!tamper_results[0].1, "Tampering must be detected");

    // Restore original
    tokio::fs::write(manifest_path, &manifest_content)
        .await
        .unwrap();
    let restore_results = mhl::verify_chain(&history).await.unwrap();
    assert!(restore_results[0].1, "Restored manifest must validate");
}
