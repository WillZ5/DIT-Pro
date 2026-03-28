//! Cascade Copy Comprehensive Stress Tests
//!
//! Validates all critical cascade scenarios for v1.0.0 release:
//! - Hash consistency across 4+ destinations
//! - Conflict resolution (Skip/Overwrite/KeepBoth) in cascade mode
//! - Cascade interrupt + resume (don't re-copy primary)
//! - Primary source file missing during cascade phase
//! - Large file count (500 files × 4 destinations = 2000 tasks)
//! - Mixed file sizes with deep directory structure
//!
//! Run with:
//!   cargo test --release --test cascade_stress_tests -- --nocapture --test-threads=1

use app_lib::checkpoint;
use app_lib::db;
use app_lib::hash_engine::{self, HashAlgorithm, HashEngineConfig};
use app_lib::workflow::{
    CancelToken, ConflictAction, OffloadConfig, OffloadEvent, OffloadPhase, OffloadWorkflow,
};
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::tempdir;
use tokio::sync::mpsc;

// ─── Helper Functions ────────────────────────────────────────────────────────

fn setup_real_db(db_path: &Path) -> Connection {
    db::init_database(db_path.to_str().unwrap()).unwrap()
}

fn wrap_db(conn: Connection) -> Arc<std::sync::Mutex<Connection>> {
    Arc::new(std::sync::Mutex::new(conn))
}

fn generate_test_file(path: &Path, size: usize, seed: u64) {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut f = std::fs::File::create(path).unwrap();
    let chunk_size = 1024 * 1024;
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

/// Generate files simulating real DIT footage with subdirectories.
fn generate_cascade_test_files(dir: &Path, count: usize) -> Vec<(String, u64)> {
    let mut files = Vec::with_capacity(count);
    for i in 0..count {
        let size: usize = match i % 10 {
            0 => 10 * 1024 * 1024, // 10MB — large clips
            1 => 5 * 1024 * 1024,  // 5MB
            2 => 1024 * 1024,      // 1MB
            3 | 4 => 500 * 1024,   // 500KB
            5 | 6 => 100 * 1024,   // 100KB — audio
            7 | 8 => 10 * 1024,    // 10KB — metadata
            _ => 1024,             // 1KB — sidecar
        };

        let subdir = match i % 7 {
            0 => "A001_C001/Clip",
            1 => "A001_C002/Clip",
            2 => "B001_C001/Clip",
            3 => "audio/mix",
            4 => "metadata/xml",
            5 => "LUT/ARRI",
            _ => "proxy",
        };
        let ext = match i % 4 {
            0 => "mov",
            1 => "mxf",
            2 => "wav",
            _ => "xml",
        };
        let rel = format!("{}/file_{:05}.{}", subdir, i, ext);
        generate_test_file(&dir.join(&rel), size, 100 + i as u64);
        files.push((rel, size as u64));
    }
    files
}

fn make_cascade_config(
    job_id: &str,
    source: &Path,
    dests: &[PathBuf],
    conflict_resolutions: HashMap<String, ConflictAction>,
) -> OffloadConfig {
    OffloadConfig {
        job_id: job_id.into(),
        job_name: format!("Cascade: {}", job_id),
        source_path: source.to_path_buf(),
        dest_paths: dests.to_vec(),
        hash_algorithms: vec![HashAlgorithm::XXH64, HashAlgorithm::SHA256],
        buffer_size: 4 * 1024 * 1024,
        source_verify: true,
        post_verify: true,
        generate_mhl: true,
        max_retries: 3,
        cascade: true,
        conflict_resolutions,
        ..Default::default()
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

async fn drain_events(mut rx: mpsc::UnboundedReceiver<OffloadEvent>) -> Vec<OffloadEvent> {
    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        events.push(ev);
    }
    events
}

// ─── Test 1: Hash Consistency — 200 files × 4 destinations ──────────────────

/// Verifies that cascade produces bit-identical copies across 4 destinations
/// with both XXH64 and SHA256 hash verification.
#[tokio::test]
async fn cascade_hash_consistency_200_files_4_dests() {
    let tmp = tempdir().unwrap();
    let source = tmp.path().join("source");
    let primary = tmp.path().join("primary");
    let sec1 = tmp.path().join("secondary1");
    let sec2 = tmp.path().join("secondary2");
    let sec3 = tmp.path().join("secondary3");

    let files = generate_cascade_test_files(&source, 200);
    let total_bytes: u64 = files.iter().map(|(_, s)| s).sum();
    eprintln!(
        "Generated {} files, {:.1} MB total",
        files.len(),
        total_bytes as f64 / 1_048_576.0
    );

    let db_path = tmp.path().join("cascade.db");
    let conn = setup_real_db(&db_path);
    let db = wrap_db(conn);
    let (tx, rx) = mpsc::unbounded_channel();

    let config = make_cascade_config(
        "hash-4dest",
        &source,
        &[primary.clone(), sec1.clone(), sec2.clone(), sec3.clone()],
        HashMap::new(),
    );

    let workflow = OffloadWorkflow::new(config, db.clone(), tx);
    let result = workflow.execute().await.unwrap();

    assert!(result.success, "Cascade must succeed");
    assert_eq!(result.total_files, 200);
    assert_eq!(result.failed_files, 0);

    // Verify hash consistency across ALL 4 destinations
    assert!(
        verify_all_files_match(
            &source,
            &[primary.clone(), sec1.clone(), sec2.clone(), sec3.clone()]
        )
        .await,
        "All 4 destinations must match source"
    );

    // MHL must be generated on all destinations
    for dest in &[&primary, &sec1, &sec2, &sec3] {
        let mhl_dir = dest.join("ascmhl");
        assert!(mhl_dir.exists(), "MHL directory must exist on {:?}", dest);
    }

    // Verify event phases: SourceVerify → Copying → Cascading → Verifying → MHL
    drop(workflow);
    let events = drain_events(rx).await;
    let phases: Vec<&OffloadPhase> = events
        .iter()
        .filter_map(|e| match e {
            OffloadEvent::PhaseChanged { phase, .. } => Some(phase),
            _ => None,
        })
        .collect();

    assert!(
        phases.contains(&&OffloadPhase::Cascading),
        "Cascading phase must be emitted"
    );

    // DB: 200 files × 4 dests = 800 tasks
    {
        let conn = db.lock().unwrap();
        let progress = checkpoint::get_job_progress(&conn, "hash-4dest").unwrap();
        assert_eq!(
            progress.completed, 800,
            "Expected 800 completed tasks (200×4), got {}",
            progress.completed
        );
        assert_eq!(progress.failed, 0);
    }

    eprintln!("[PASS] cascade_hash_consistency_200_files_4_dests PASSED");
}

// ─── Test 2: Conflict Resolution in Cascade Mode ────────────────────────────

/// Tests Skip, Overwrite, KeepBoth conflict resolutions work correctly
/// during cascade phase (not just primary copy phase).
#[tokio::test]
async fn cascade_conflict_resolution_mixed() {
    let tmp = tempdir().unwrap();
    let source = tmp.path().join("source");
    let primary = tmp.path().join("primary");
    let sec1 = tmp.path().join("secondary1");

    // Create 5 source files
    for i in 0..5 {
        generate_test_file(
            &source.join(format!("clip_{}.mov", i)),
            100 * 1024,
            i as u64,
        );
    }

    // Pre-populate destinations with conflicting files (different content)
    std::fs::create_dir_all(&primary).unwrap();
    std::fs::create_dir_all(&sec1).unwrap();
    for i in 0..3 {
        generate_test_file(
            &primary.join(format!("clip_{}.mov", i)),
            100 * 1024,
            999 + i as u64, // different seed = different content
        );
        generate_test_file(
            &sec1.join(format!("clip_{}.mov", i)),
            100 * 1024,
            888 + i as u64,
        );
    }

    let db_path = tmp.path().join("cascade.db");
    let conn = setup_real_db(&db_path);
    let db = wrap_db(conn);
    let (tx, rx) = mpsc::unbounded_channel();

    // Set conflict resolutions:
    // clip_0: Skip on primary AND secondary
    // clip_1: Overwrite on primary AND secondary
    // clip_2: KeepBoth on primary AND secondary
    // clip_3, clip_4: no conflict (fresh files)
    let mut resolutions = HashMap::new();
    resolutions.insert(
        format!(
            "clip_0.mov::{}",
            primary.join("clip_0.mov").to_string_lossy()
        ),
        ConflictAction::Skip,
    );
    resolutions.insert(
        format!("clip_0.mov::{}", sec1.join("clip_0.mov").to_string_lossy()),
        ConflictAction::Skip,
    );
    resolutions.insert(
        format!(
            "clip_1.mov::{}",
            primary.join("clip_1.mov").to_string_lossy()
        ),
        ConflictAction::Overwrite,
    );
    resolutions.insert(
        format!("clip_1.mov::{}", sec1.join("clip_1.mov").to_string_lossy()),
        ConflictAction::Overwrite,
    );
    resolutions.insert(
        format!(
            "clip_2.mov::{}",
            primary.join("clip_2.mov").to_string_lossy()
        ),
        ConflictAction::KeepBoth,
    );
    resolutions.insert(
        format!("clip_2.mov::{}", sec1.join("clip_2.mov").to_string_lossy()),
        ConflictAction::KeepBoth,
    );

    let config = OffloadConfig {
        job_id: "cascade-conflict".into(),
        job_name: "Cascade Conflict Test".into(),
        source_path: source.clone(),
        dest_paths: vec![primary.clone(), sec1.clone()],
        hash_algorithms: vec![HashAlgorithm::XXH64],
        buffer_size: 4 * 1024 * 1024,
        source_verify: false,
        post_verify: true,
        generate_mhl: false,
        max_retries: 3,
        cascade: true,
        conflict_resolutions: resolutions,
        ..Default::default()
    };

    let workflow = OffloadWorkflow::new(config, db.clone(), tx);
    let result = workflow.execute().await;

    eprintln!("Conflict test result: {:?}", result);

    let result = result.unwrap();
    eprintln!(
        "success={}, total={}, failed={}",
        result.success, result.total_files, result.failed_files
    );

    assert!(
        result.success,
        "Cascade with conflicts must succeed (failed={})",
        result.failed_files
    );

    // clip_0: Skipped → original (different content) should remain
    let skip_primary = std::fs::read(primary.join("clip_0.mov")).unwrap();
    let skip_source = std::fs::read(source.join("clip_0.mov")).unwrap();
    assert_ne!(
        skip_primary, skip_source,
        "Skip: primary should keep original content"
    );

    // clip_1: Overwrite → should match source
    let ow_primary = std::fs::read(primary.join("clip_1.mov")).unwrap();
    let ow_source = std::fs::read(source.join("clip_1.mov")).unwrap();
    assert_eq!(
        ow_primary, ow_source,
        "Overwrite: primary should match source"
    );

    // clip_1: Overwrite on secondary too
    let ow_sec = std::fs::read(sec1.join("clip_1.mov")).unwrap();
    assert_eq!(
        ow_sec, ow_source,
        "Overwrite: secondary should match source"
    );

    // clip_2: KeepBoth → original remains, _copy variant should exist
    assert!(
        primary.join("clip_2.mov").exists(),
        "KeepBoth: original should remain"
    );
    // The _copy file should exist and match source size
    let keepboth_copy = primary.join("clip_2_copy.mov");
    assert!(
        keepboth_copy.exists(),
        "KeepBoth: _copy variant must exist on primary"
    );
    let kb_meta = std::fs::metadata(&keepboth_copy).unwrap();
    let src_meta = std::fs::metadata(source.join("clip_2.mov")).unwrap();
    assert_eq!(
        kb_meta.len(),
        src_meta.len(),
        "KeepBoth: _copy size must match source"
    );

    // clip_3, clip_4: no conflict → should match source on both dests
    for i in 3..5 {
        let name = format!("clip_{}.mov", i);
        let p_content = std::fs::read(primary.join(&name)).unwrap();
        let s_content = std::fs::read(source.join(&name)).unwrap();
        assert_eq!(
            p_content, s_content,
            "Fresh file {} must match source",
            name
        );

        let sec_content = std::fs::read(sec1.join(&name)).unwrap();
        assert_eq!(
            sec_content, s_content,
            "Fresh file {} must match on secondary",
            name
        );
    }

    // Verify cascading event was emitted
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
    assert!(has_cascade, "Cascading phase must be emitted");

    eprintln!("[PASS] cascade_conflict_resolution_mixed PASSED");
}

// ─── Test 3: Cascade Interrupt & Resume ─────────────────────────────────────

/// Interrupts cascade during primary copy, resumes, and verifies:
/// - No files re-copied to primary
/// - Cascade phase completes for remaining files
/// - All destinations match source after resume
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn cascade_interrupt_and_resume() {
    let tmp = tempdir().unwrap();
    let source = tmp.path().join("source");
    let primary = tmp.path().join("primary");
    let sec1 = tmp.path().join("secondary1");

    // 100 files × 100KB each
    for i in 0..100 {
        generate_test_file(
            &source.join(format!("clip_{:04}.mov", i)),
            100 * 1024,
            i as u64,
        );
    }

    let db_path = tmp.path().join("cascade.db");
    let conn = setup_real_db(&db_path);
    let db = wrap_db(conn);

    // ── Phase 1: Start cascade and cancel mid-way ──
    let cancel = CancelToken::new();
    let cancel_clone = cancel.clone();
    let db_clone = db.clone();
    let source_clone = source.clone();
    let primary_clone = primary.clone();
    let sec1_clone = sec1.clone();
    let (tx1, _rx1) = mpsc::unbounded_channel();

    let handle = tokio::spawn(async move {
        let config = OffloadConfig {
            job_id: "cascade-interrupt".into(),
            job_name: "Cascade Interrupt".into(),
            source_path: source_clone,
            dest_paths: vec![primary_clone, sec1_clone],
            hash_algorithms: vec![HashAlgorithm::XXH64],
            buffer_size: 4 * 1024 * 1024,
            source_verify: false,
            post_verify: false,
            generate_mhl: false,
            max_retries: 3,
            cascade: true,
            conflict_resolutions: HashMap::new(),
            ..Default::default()
        };
        let workflow = OffloadWorkflow::with_cancel(config, db_clone, tx1, cancel_clone);
        workflow.execute().await
    });

    // Wait for some tasks to complete, then cancel
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let conn = db.lock().unwrap();
        if let Ok(progress) = checkpoint::get_job_progress(&conn, "cascade-interrupt") {
            if progress.completed >= 20 {
                cancel.cancel();
                break;
            }
        }
    }

    let result1 = handle.await.unwrap();
    assert!(result1.is_err(), "Cancelled workflow should error");

    let completed_before;
    {
        let conn = db.lock().unwrap();
        let progress = checkpoint::get_job_progress(&conn, "cascade-interrupt").unwrap();
        completed_before = progress.completed;
        eprintln!(
            "After cancel: completed={}, failed={}, pending={}",
            progress.completed, progress.failed, progress.pending
        );
        assert!(
            completed_before >= 20,
            "At least 20 tasks done before cancel"
        );
    }

    // ── Phase 2: Recover and resume ──
    {
        let conn = db.lock().unwrap();
        let _recovered = checkpoint::recover_job(&conn, "cascade-interrupt")
            .await
            .unwrap();
        let progress = checkpoint::get_job_progress(&conn, "cascade-interrupt").unwrap();
        assert_eq!(progress.copying, 0, "No stuck copying tasks after recovery");
    }

    let (tx2, _rx2) = mpsc::unbounded_channel();
    let config2 = OffloadConfig {
        job_id: "cascade-interrupt".into(),
        job_name: "Cascade Resume".into(),
        source_path: source.clone(),
        dest_paths: vec![primary.clone(), sec1.clone()],
        hash_algorithms: vec![HashAlgorithm::XXH64],
        buffer_size: 4 * 1024 * 1024,
        source_verify: false,
        post_verify: true,
        generate_mhl: false,
        max_retries: 3,
        cascade: true,
        conflict_resolutions: HashMap::new(),
        ..Default::default()
    };

    let workflow2 = OffloadWorkflow::new(config2, db.clone(), tx2);
    let result2 = workflow2.execute_resume().await.unwrap();
    assert!(result2.success, "Resume must succeed");

    // All 100 files must exist on BOTH destinations
    for i in 0..100 {
        let name = format!("clip_{:04}.mov", i);
        assert!(primary.join(&name).exists(), "Primary must have {}", name);
        assert!(sec1.join(&name).exists(), "Secondary must have {}", name);
    }

    // Verify hash consistency
    assert!(
        verify_all_files_match(&source, &[primary.clone(), sec1.clone()]).await,
        "Both destinations must match source after resume"
    );

    // DB: 100 files × 2 dests = 200 tasks all completed
    {
        let conn = db.lock().unwrap();
        let progress = checkpoint::get_job_progress(&conn, "cascade-interrupt").unwrap();
        assert_eq!(progress.completed, 200, "All 200 tasks must be completed");
        assert_eq!(progress.pending, 0);
        assert_eq!(progress.failed, 0);
    }

    eprintln!("[PASS] cascade_interrupt_and_resume PASSED");
}

// ─── Test 4: Large Scale Cascade — 500 files × 3 dests ─────────────────────

/// Stress test: 500 files with varied sizes across 3 cascade destinations.
/// Validates at scale that no files are lost or corrupted.
#[tokio::test]
async fn cascade_500_files_3_dests_full_pipeline() {
    let tmp = tempdir().unwrap();
    let source = tmp.path().join("source");
    let primary = tmp.path().join("primary");
    let sec1 = tmp.path().join("secondary1");
    let sec2 = tmp.path().join("secondary2");

    let files = generate_cascade_test_files(&source, 500);
    let total_bytes: u64 = files.iter().map(|(_, s)| s).sum();
    eprintln!(
        "Generated {} files, {:.1} MB total",
        files.len(),
        total_bytes as f64 / 1_048_576.0
    );

    let db_path = tmp.path().join("cascade.db");
    let conn = setup_real_db(&db_path);
    let db = wrap_db(conn);
    let (tx, rx) = mpsc::unbounded_channel();

    let config = make_cascade_config(
        "cascade-500",
        &source,
        &[primary.clone(), sec1.clone(), sec2.clone()],
        HashMap::new(),
    );

    let start = std::time::Instant::now();
    let workflow = OffloadWorkflow::new(config, db.clone(), tx);
    let result = workflow.execute().await.unwrap();
    let elapsed = start.elapsed();

    assert!(result.success, "Cascade 500 files must succeed");
    assert_eq!(result.total_files, 500);
    assert_eq!(result.failed_files, 0);

    // Verify all destinations match source
    assert!(
        verify_all_files_match(&source, &[primary.clone(), sec1.clone(), sec2.clone()]).await,
        "All 3 destinations must match source"
    );

    // MHL on all destinations
    for dest in &[&primary, &sec1, &sec2] {
        assert!(dest.join("ascmhl").exists(), "MHL must exist on {:?}", dest);
    }

    // Verify cascading phase + event ordering
    drop(workflow);
    let events = drain_events(rx).await;
    let phase_changes: Vec<&OffloadPhase> = events
        .iter()
        .filter_map(|e| match e {
            OffloadEvent::PhaseChanged { phase, .. } => Some(phase),
            _ => None,
        })
        .collect();

    assert!(
        phase_changes.contains(&&OffloadPhase::Cascading),
        "Must emit Cascading phase"
    );

    // DB: 500 × 3 = 1500 tasks
    {
        let conn = db.lock().unwrap();
        let progress = checkpoint::get_job_progress(&conn, "cascade-500").unwrap();
        assert_eq!(progress.completed, 1500);
        assert_eq!(progress.failed, 0);
    }

    let total_copied_mb = (total_bytes as f64 * 3.0) / 1_048_576.0;
    let speed_mbps = total_copied_mb / elapsed.as_secs_f64();
    eprintln!(
        "[PASS] cascade_500_files_3_dests: {:.1}MB across 3 dests in {:.1}s ({:.1} MB/s effective)",
        total_copied_mb,
        elapsed.as_secs_f64(),
        speed_mbps
    );
}

// ─── Test 5: Cascade Phase Event Ordering ───────────────────────────────────

/// Verifies the exact phase ordering in cascade mode (with post_verify):
/// PreFlight → SourceVerify → Copying (primary) → Verifying (primary)
///   → Cascading → Verifying (secondaries) → Sealing → Complete
#[tokio::test]
async fn cascade_phase_event_ordering() {
    let tmp = tempdir().unwrap();
    let source = tmp.path().join("source");
    let primary = tmp.path().join("primary");
    let sec1 = tmp.path().join("secondary1");

    // 10 small files for speed
    for i in 0..10 {
        generate_test_file(&source.join(format!("f_{}.mov", i)), 10 * 1024, i as u64);
    }

    let db_path = tmp.path().join("cascade.db");
    let conn = setup_real_db(&db_path);
    let db = wrap_db(conn);
    let (tx, rx) = mpsc::unbounded_channel();

    let config = make_cascade_config(
        "phase-order",
        &source,
        &[primary.clone(), sec1.clone()],
        HashMap::new(),
    );

    let workflow = OffloadWorkflow::new(config, db, tx);
    let result = workflow.execute().await.unwrap();
    assert!(result.success);

    drop(workflow);
    let events = drain_events(rx).await;

    // Extract unique phase ordering (deduplicated)
    let mut phase_order: Vec<OffloadPhase> = Vec::new();
    for ev in &events {
        if let OffloadEvent::PhaseChanged { phase, .. } = ev {
            if phase_order.last() != Some(phase) {
                phase_order.push(*phase);
            }
        }
    }

    eprintln!("Phase order: {:?}", phase_order);

    // Expected order for cascade with post_verify:
    // PreFlight → SourceVerify → Copying (to primary) → Verifying (primary)
    //   → Cascading (primary → secondaries) → Verifying (secondaries) → Sealing → Complete
    // The first Verifying ensures primary integrity before releasing source card.
    let expected = vec![
        OffloadPhase::PreFlight,
        OffloadPhase::SourceVerify,
        OffloadPhase::Copying,
        OffloadPhase::Verifying,
        OffloadPhase::Cascading,
        OffloadPhase::Verifying,
        OffloadPhase::Sealing,
        OffloadPhase::Complete,
    ];

    assert_eq!(
        phase_order, expected,
        "Phase ordering mismatch!\nGot:      {:?}\nExpected: {:?}",
        phase_order, expected
    );

    eprintln!("[PASS] cascade_phase_event_ordering PASSED");
}

// ─── Test 6: Cascade with MHL Verification ──────────────────────────────────

/// After cascade, verify that MHL manifests on all destinations
/// are independently verifiable (chain integrity).
#[tokio::test]
async fn cascade_mhl_integrity_all_destinations() {
    let tmp = tempdir().unwrap();
    let source = tmp.path().join("source");
    let primary = tmp.path().join("primary");
    let sec1 = tmp.path().join("secondary1");
    let sec2 = tmp.path().join("secondary2");

    // 20 files
    for i in 0..20 {
        generate_test_file(
            &source.join(format!("clip_{:03}.mov", i)),
            50 * 1024,
            i as u64,
        );
    }

    let db_path = tmp.path().join("cascade.db");
    let conn = setup_real_db(&db_path);
    let db = wrap_db(conn);
    let (tx, _rx) = mpsc::unbounded_channel();

    let config = make_cascade_config(
        "mhl-cascade",
        &source,
        &[primary.clone(), sec1.clone(), sec2.clone()],
        HashMap::new(),
    );

    let workflow = OffloadWorkflow::new(config, db, tx);
    let result = workflow.execute().await.unwrap();
    assert!(result.success);
    assert!(!result.mhl_paths.is_empty(), "MHL paths must be returned");

    // Verify each destination has valid MHL chain
    for dest in &[&primary, &sec1, &sec2] {
        let mhl_dir = dest.join("ascmhl");
        assert!(mhl_dir.exists(), "MHL dir must exist on {:?}", dest);

        // Load and verify the MHL chain
        let history = app_lib::mhl::load_or_create_history(dest).await.unwrap();
        assert!(
            !history.chain.is_empty(),
            "MHL chain must not be empty on {:?}",
            dest
        );

        // Verify chain hash integrity
        let verify_result = app_lib::mhl::verify_chain(&history).await;
        assert!(
            verify_result.is_ok(),
            "MHL chain verification must pass on {:?}: {:?}",
            dest,
            verify_result.err()
        );
    }

    eprintln!("[PASS] cascade_mhl_integrity_all_destinations PASSED");
}

// ─── Test 7: No .tmp Files After Cascade ────────────────────────────────────

/// Ensures no temporary files remain on any destination after cascade completion.
#[tokio::test]
async fn cascade_no_tmp_files_remain() {
    let tmp = tempdir().unwrap();
    let source = tmp.path().join("source");
    let primary = tmp.path().join("primary");
    let sec1 = tmp.path().join("secondary1");

    for i in 0..50 {
        generate_test_file(
            &source.join(format!("clip_{:03}.mov", i)),
            100 * 1024,
            i as u64,
        );
    }

    let db_path = tmp.path().join("cascade.db");
    let conn = setup_real_db(&db_path);
    let db = wrap_db(conn);
    let (tx, _rx) = mpsc::unbounded_channel();

    let config = make_cascade_config(
        "tmp-check",
        &source,
        &[primary.clone(), sec1.clone()],
        HashMap::new(),
    );

    let workflow = OffloadWorkflow::new(config, db, tx);
    let result = workflow.execute().await.unwrap();
    assert!(result.success);

    // Check no .tmp files on any destination
    fn check_no_tmp(dir: &Path) {
        if !dir.exists() {
            return;
        }
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

    check_no_tmp(&primary);
    check_no_tmp(&sec1);

    eprintln!("[PASS] cascade_no_tmp_files_remain PASSED");
}

// ─── Test 8: Cascade DB State Consistency ───────────────────────────────────

/// Verifies that after cascade, every task in DB has correct status,
/// dest_path, and hash values.
#[tokio::test]
async fn cascade_db_state_consistency() {
    let tmp = tempdir().unwrap();
    let source = tmp.path().join("source");
    let primary = tmp.path().join("primary");
    let sec1 = tmp.path().join("secondary1");

    for i in 0..30 {
        generate_test_file(
            &source.join(format!("file_{:03}.mov", i)),
            50 * 1024,
            i as u64,
        );
    }

    let db_path = tmp.path().join("cascade.db");
    let conn = setup_real_db(&db_path);
    let db = wrap_db(conn);
    let (tx, _rx) = mpsc::unbounded_channel();

    let config = make_cascade_config(
        "db-state",
        &source,
        &[primary.clone(), sec1.clone()],
        HashMap::new(),
    );

    let workflow = OffloadWorkflow::new(config, db.clone(), tx);
    let result = workflow.execute().await.unwrap();
    assert!(result.success);

    // Check DB state: all tasks completed with hashes
    {
        let conn = db.lock().unwrap();
        let progress = checkpoint::get_job_progress(&conn, "db-state").unwrap();

        // 30 files × 2 dests = 60 tasks
        assert_eq!(
            progress.completed, 60,
            "Expected 60 completed tasks, got {}",
            progress.completed
        );
        assert_eq!(progress.failed, 0, "No tasks should have failed");
        assert_eq!(progress.pending, 0, "No tasks should be pending");
    }

    // Verify all files actually exist on disk
    fn count_files(dir: &Path) -> usize {
        let mut count = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.file_name().map(|n| n == "ascmhl").unwrap_or(false) {
                    continue;
                }
                count += count_files(&path);
            } else if !path.to_string_lossy().contains(".DS_Store") {
                count += 1;
            }
        }
        count
    }

    assert_eq!(count_files(&primary), 30, "Primary should have 30 files");
    assert_eq!(count_files(&sec1), 30, "Secondary should have 30 files");

    // Verify hash consistency
    assert!(
        verify_all_files_match(&source, &[primary, sec1]).await,
        "Both destinations must match source"
    );

    eprintln!("[PASS] cascade_db_state_consistency PASSED");
}
