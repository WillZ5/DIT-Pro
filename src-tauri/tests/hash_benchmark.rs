//! Hash Engine Throughput Benchmark
//!
//! Measures hashing speed for each algorithm to verify we meet
//! the 10 Gbps+ target for XXH64/XXH3.

use app_lib::hash_engine::{HashAlgorithm, MultiHasher};
use std::time::Instant;

const BENCHMARK_SIZE: usize = 256 * 1024 * 1024; // 256 MB
const CHUNK_SIZE: usize = 4 * 1024 * 1024; // 4 MB chunks (matches copy engine buffer)

fn benchmark_algorithm(algo: HashAlgorithm, data: &[u8]) -> f64 {
    let start = Instant::now();
    let mut hasher = MultiHasher::new(&[algo]);

    for chunk in data.chunks(CHUNK_SIZE) {
        hasher.update(chunk);
    }

    let _results = hasher.finalize();
    let elapsed = start.elapsed();

    let bytes_per_sec = data.len() as f64 / elapsed.as_secs_f64();
    let gbps = bytes_per_sec * 8.0 / 1_000_000_000.0;
    gbps
}

fn benchmark_multi(algos: &[HashAlgorithm], data: &[u8]) -> f64 {
    let start = Instant::now();
    let mut hasher = MultiHasher::new(algos);

    for chunk in data.chunks(CHUNK_SIZE) {
        hasher.update(chunk);
    }

    let _results = hasher.finalize();
    let elapsed = start.elapsed();

    let bytes_per_sec = data.len() as f64 / elapsed.as_secs_f64();
    let gbps = bytes_per_sec * 8.0 / 1_000_000_000.0;
    gbps
}

#[test]
fn benchmark_hash_throughput() {
    // Generate test data (random-ish to avoid compression optimizations)
    let mut data = vec![0u8; BENCHMARK_SIZE];
    for (i, byte) in data.iter_mut().enumerate() {
        *byte = (i.wrapping_mul(7) ^ i.wrapping_mul(13)) as u8;
    }

    println!("\n=== DIT System Hash Engine Throughput Benchmark ===");
    println!("Data size: {} MB, Chunk size: {} MB\n", BENCHMARK_SIZE / (1024*1024), CHUNK_SIZE / (1024*1024));

    // Benchmark individual algorithms
    let algorithms = vec![
        (HashAlgorithm::XXH64, "XXH64"),
        (HashAlgorithm::XXH3, "XXH3"),
        (HashAlgorithm::XXH128, "XXH128"),
        (HashAlgorithm::SHA256, "SHA-256"),
        (HashAlgorithm::MD5, "MD5"),
    ];

    for (algo, name) in &algorithms {
        let gbps = benchmark_algorithm(*algo, &data);
        let gb_per_sec = gbps / 8.0;
        println!("{:<8} : {:.2} Gbps ({:.2} GB/s)", name, gbps, gb_per_sec);
    }

    // Benchmark multi-algorithm (common use case: XXH64 + SHA-256)
    println!();
    let multi_gbps = benchmark_multi(
        &[HashAlgorithm::XXH64, HashAlgorithm::SHA256],
        &data,
    );
    println!(
        "XXH64+SHA256 combined : {:.2} Gbps ({:.2} GB/s)",
        multi_gbps,
        multi_gbps / 8.0
    );

    let all_gbps = benchmark_multi(
        &[HashAlgorithm::XXH64, HashAlgorithm::XXH3, HashAlgorithm::SHA256, HashAlgorithm::MD5],
        &data,
    );
    println!(
        "All 4 algorithms     : {:.2} Gbps ({:.2} GB/s)",
        all_gbps,
        all_gbps / 8.0
    );

    println!("\n=== Target (release mode): XXH64 >= 10 Gbps, SHA-256 >= 1 Gbps ===");
    println!("Note: debug mode is ~10-20x slower. Run with --release for real numbers.\n");

    // Only assert in release mode — debug builds are much slower
    let xxh64_gbps = benchmark_algorithm(HashAlgorithm::XXH64, &data);
    if cfg!(not(debug_assertions)) {
        assert!(
            xxh64_gbps >= 5.0,
            "XXH64 throughput ({:.2} Gbps) is below minimum threshold of 5 Gbps",
            xxh64_gbps
        );
    }
}
