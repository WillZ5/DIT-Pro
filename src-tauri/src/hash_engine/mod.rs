//! Hash Engine — Multi-algorithm parallel hash computation.
//!
//! Supported algorithms:
//! - XXH64 (default, ~10+ GB/s)
//! - XXH3 / XXH128 (~12+ GB/s)
//! - SHA-256 (cryptographic)
//! - MD5 (legacy compatibility)
//! - C4ID (film industry content identifier, future)

use serde::{Deserialize, Serialize};

/// Supported hash algorithms
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum HashAlgorithm {
    XXH64,
    XXH3,
    XXH128,
    SHA256,
    MD5,
    // C4ID — to be implemented
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

/// Result of hashing a file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HashResult {
    pub algorithm: HashAlgorithm,
    pub hex_digest: String,
}

/// Configuration for hash computation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HashEngineConfig {
    /// Which algorithms to compute (can be multiple simultaneously)
    pub algorithms: Vec<HashAlgorithm>,
}

impl Default for HashEngineConfig {
    fn default() -> Self {
        Self {
            algorithms: vec![HashAlgorithm::XXH64],
        }
    }
}
