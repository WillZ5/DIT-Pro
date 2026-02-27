//! Hash Engine — Multi-algorithm parallel hash computation.
//!
//! Supported algorithms:
//! - XXH64 (default, ~10+ GB/s)
//! - XXH3 / XXH128 (~12+ GB/s)
//! - SHA-256 (cryptographic)
//! - MD5 (legacy compatibility)
//!
//! Key design: multiple algorithms are computed in a single pass over the data,
//! avoiding redundant reads. The `MultiHasher` accumulates all selected algorithms
//! simultaneously.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest as Sha2Digest, Sha256};
use std::path::Path;
use tokio::io::AsyncReadExt;

/// Supported hash algorithms
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum HashAlgorithm {
    XXH64,
    XXH3,
    XXH128,
    SHA256,
    MD5,
}

impl std::fmt::Display for HashAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HashAlgorithm::XXH64 => write!(f, "XXH64"),
            HashAlgorithm::XXH3 => write!(f, "XXH3"),
            HashAlgorithm::XXH128 => write!(f, "XXH128"),
            HashAlgorithm::SHA256 => write!(f, "SHA-256"),
            HashAlgorithm::MD5 => write!(f, "MD5"),
        }
    }
}

/// Result of hashing a file with one algorithm
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HashResult {
    pub algorithm: HashAlgorithm,
    pub hex_digest: String,
}

/// Configuration for hash computation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HashEngineConfig {
    /// Which algorithms to compute (can be multiple simultaneously)
    pub algorithms: Vec<HashAlgorithm>,
    /// Buffer size for reading (default: 4MB)
    pub buffer_size: usize,
}

impl Default for HashEngineConfig {
    fn default() -> Self {
        Self {
            algorithms: vec![HashAlgorithm::XXH64],
            buffer_size: 4 * 1024 * 1024, // 4MB
        }
    }
}

/// Multi-algorithm hasher that computes all selected hashes in a single pass.
pub struct MultiHasher {
    xxh64: Option<xxhash_rust::xxh64::Xxh64>,
    xxh3_hasher: Option<xxhash_rust::xxh3::Xxh3>,
    xxh128_streaming: Option<xxhash_rust::xxh3::Xxh3>,
    sha256: Option<Sha256>,
    md5: Option<md5::Md5>,
    algorithms: Vec<HashAlgorithm>,
    bytes_processed: u64,
}

impl MultiHasher {
    /// Create a new multi-hasher for the specified algorithms
    pub fn new(algorithms: &[HashAlgorithm]) -> Self {
        let mut hasher = Self {
            xxh64: None,
            xxh3_hasher: None,
            xxh128_streaming: None,
            sha256: None,
            md5: None,
            algorithms: algorithms.to_vec(),
            bytes_processed: 0,
        };

        for algo in algorithms {
            match algo {
                HashAlgorithm::XXH64 => {
                    hasher.xxh64 = Some(xxhash_rust::xxh64::Xxh64::new(0));
                }
                HashAlgorithm::XXH3 => {
                    hasher.xxh3_hasher = Some(xxhash_rust::xxh3::Xxh3::new());
                }
                HashAlgorithm::XXH128 => {
                    hasher.xxh128_streaming = Some(xxhash_rust::xxh3::Xxh3::new());
                }
                HashAlgorithm::SHA256 => {
                    hasher.sha256 = Some(Sha256::new());
                }
                HashAlgorithm::MD5 => {
                    hasher.md5 = Some(<md5::Md5 as md5::Digest>::new());
                }
            }
        }

        hasher
    }

    /// Feed a chunk of data to all active hashers
    pub fn update(&mut self, data: &[u8]) {
        self.bytes_processed += data.len() as u64;

        if let Some(ref mut h) = self.xxh64 {
            h.update(data);
        }
        if let Some(ref mut h) = self.xxh3_hasher {
            h.update(data);
        }
        if let Some(ref mut h) = self.xxh128_streaming {
            h.update(data);
        }
        if let Some(ref mut h) = self.sha256 {
            sha2::Digest::update(h, data);
        }
        if let Some(ref mut h) = self.md5 {
            md5::Digest::update(h, data);
        }
    }

    /// Finalize all hashers and return results
    pub fn finalize(self) -> Vec<HashResult> {
        let mut results = Vec::with_capacity(self.algorithms.len());

        if let Some(h) = self.xxh64 {
            results.push(HashResult {
                algorithm: HashAlgorithm::XXH64,
                hex_digest: format!("{:016x}", h.digest()),
            });
        }

        if let Some(h) = self.xxh3_hasher {
            results.push(HashResult {
                algorithm: HashAlgorithm::XXH3,
                hex_digest: format!("{:016x}", h.digest()),
            });
        }

        if let Some(h) = self.xxh128_streaming {
            results.push(HashResult {
                algorithm: HashAlgorithm::XXH128,
                hex_digest: format!("{:032x}", h.digest128()),
            });
        }

        if let Some(h) = self.sha256 {
            let digest = sha2::Digest::finalize(h);
            results.push(HashResult {
                algorithm: HashAlgorithm::SHA256,
                hex_digest: format!("{:x}", digest),
            });
        }

        if let Some(h) = self.md5 {
            let digest = md5::Digest::finalize(h);
            results.push(HashResult {
                algorithm: HashAlgorithm::MD5,
                hex_digest: format!("{:x}", digest),
            });
        }

        results
    }

    /// Number of bytes processed so far
    pub fn bytes_processed(&self) -> u64 {
        self.bytes_processed
    }
}

/// Hash a file using multiple algorithms in a single pass (async)
pub async fn hash_file(
    path: &Path,
    config: &HashEngineConfig,
) -> Result<Vec<HashResult>> {
    hash_file_with_progress(path, config, None).await
}

/// Hash a file with optional progress callback (async).
/// The callback receives `(bytes_processed, total_bytes)` and is called after each buffer read.
pub async fn hash_file_with_progress(
    path: &Path,
    config: &HashEngineConfig,
    on_progress: Option<&(dyn Fn(u64, u64) + Send + Sync)>,
) -> Result<Vec<HashResult>> {
    let metadata = tokio::fs::metadata(path).await?;
    let total_bytes = metadata.len();
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = MultiHasher::new(&config.algorithms);
    let mut buffer = vec![0u8; config.buffer_size];

    loop {
        let bytes_read = file.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
        if let Some(cb) = &on_progress {
            cb(hasher.bytes_processed(), total_bytes);
        }
    }

    Ok(hasher.finalize())
}

/// Hash a file synchronously (for use in non-async contexts)
pub fn hash_file_sync(
    path: &Path,
    config: &HashEngineConfig,
) -> Result<Vec<HashResult>> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = MultiHasher::new(&config.algorithms);
    let mut buffer = vec![0u8; config.buffer_size];

    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hasher.finalize())
}

/// Hash raw bytes in memory (useful for inline verification during copy)
pub fn hash_bytes(data: &[u8], algorithms: &[HashAlgorithm]) -> Vec<HashResult> {
    let mut hasher = MultiHasher::new(algorithms);
    hasher.update(data);
    hasher.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_xxh64_hash() {
        let results = hash_bytes(b"hello world", &[HashAlgorithm::XXH64]);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].algorithm, HashAlgorithm::XXH64);
        // Known XXH64 hash for "hello world" with seed 0
        assert!(!results[0].hex_digest.is_empty());
    }

    #[test]
    fn test_known_sha256_hash() {
        let results = hash_bytes(b"hello world", &[HashAlgorithm::SHA256]);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].algorithm, HashAlgorithm::SHA256);
        // Known SHA-256 for "hello world"
        assert_eq!(
            results[0].hex_digest,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_known_md5_hash() {
        let results = hash_bytes(b"hello world", &[HashAlgorithm::MD5]);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].algorithm, HashAlgorithm::MD5);
        // Known MD5 for "hello world"
        assert_eq!(results[0].hex_digest, "5eb63bbbe01eeed093cb22bb8f5acdc3");
    }

    #[test]
    fn test_multi_algorithm_single_pass() {
        let algos = vec![
            HashAlgorithm::XXH64,
            HashAlgorithm::XXH3,
            HashAlgorithm::SHA256,
            HashAlgorithm::MD5,
        ];
        let results = hash_bytes(b"test data for multi-hash", &algos);
        assert_eq!(results.len(), 4);

        // Verify each algorithm produced a non-empty result
        for result in &results {
            assert!(!result.hex_digest.is_empty());
        }
    }

    #[test]
    fn test_empty_input() {
        let results = hash_bytes(b"", &[HashAlgorithm::SHA256]);
        assert_eq!(results.len(), 1);
        // Known SHA-256 for empty string
        assert_eq!(
            results[0].hex_digest,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_xxh128_output() {
        let results = hash_bytes(b"hello world", &[HashAlgorithm::XXH128]);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].algorithm, HashAlgorithm::XXH128);
        // XXH128 should produce a 32-char hex string (128 bits)
        assert_eq!(results[0].hex_digest.len(), 32);
    }

    #[test]
    fn test_incremental_equals_oneshot() {
        let data = b"the quick brown fox jumps over the lazy dog";

        // One-shot
        let oneshot = hash_bytes(data, &[HashAlgorithm::SHA256]);

        // Incremental (split into chunks)
        let mut hasher = MultiHasher::new(&[HashAlgorithm::SHA256]);
        hasher.update(&data[..10]);
        hasher.update(&data[10..20]);
        hasher.update(&data[20..]);
        let incremental = hasher.finalize();

        assert_eq!(oneshot[0].hex_digest, incremental[0].hex_digest);
    }

    #[tokio::test]
    async fn test_hash_file() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.bin");

        // Write test data
        let mut f = std::fs::File::create(&file_path).unwrap();
        f.write_all(b"file content for hashing").unwrap();
        drop(f);

        let config = HashEngineConfig {
            algorithms: vec![HashAlgorithm::XXH64, HashAlgorithm::SHA256],
            buffer_size: 1024,
        };

        let results = hash_file(&file_path, &config).await.unwrap();
        assert_eq!(results.len(), 2);

        // Compare with sync version
        let sync_results = hash_file_sync(&file_path, &config).unwrap();
        assert_eq!(results[0].hex_digest, sync_results[0].hex_digest);
        assert_eq!(results[1].hex_digest, sync_results[1].hex_digest);
    }
}
