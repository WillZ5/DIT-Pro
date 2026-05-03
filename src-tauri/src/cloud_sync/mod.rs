//! Cloud Sync Module — Abstract cloud storage access using OpenDAL.
//!
//! Supports S3-compatible storage and WebDAV (for Alist/Baidu/Aliyun compatibility).

use anyhow::{Context, Result};
use opendal::Operator;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum CloudProvider {
    S3 {
        endpoint: String,
        region: String,
        bucket: String,
        access_key: String,
        secret_key: String,
    },
    WebDav {
        endpoint: String,
        username: String,
        password: String,
        root: String,
    },
}

impl Default for CloudProvider {
    fn default() -> Self {
        Self::S3 {
            endpoint: String::new(),
            region: String::new(),
            bucket: String::new(),
            access_key: String::new(),
            secret_key: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s3_provider_accepts_frontend_camel_case_keys() {
        let provider: CloudProvider = serde_json::from_value(serde_json::json!({
            "type": "s3",
            "endpoint": "https://s3.example.com",
            "region": "us-east-1",
            "bucket": "dit-pro",
            "accessKey": "AKIA_TEST",
            "secretKey": "SECRET_TEST"
        }))
        .expect("frontend S3 provider should deserialize");

        let CloudProvider::S3 {
            endpoint,
            region,
            bucket,
            access_key,
            secret_key,
        } = provider
        else {
            panic!("expected S3 provider");
        };

        assert_eq!(endpoint, "https://s3.example.com");
        assert_eq!(region, "us-east-1");
        assert_eq!(bucket, "dit-pro");
        assert_eq!(access_key, "AKIA_TEST");
        assert_eq!(secret_key, "SECRET_TEST");
    }

    #[test]
    fn s3_provider_serializes_frontend_camel_case_keys() {
        let provider = CloudProvider::S3 {
            endpoint: "https://s3.example.com".to_string(),
            region: "us-east-1".to_string(),
            bucket: "dit-pro".to_string(),
            access_key: "AKIA_TEST".to_string(),
            secret_key: "SECRET_TEST".to_string(),
        };

        let value = serde_json::to_value(provider).expect("S3 provider should serialize");
        assert_eq!(value["type"], "s3");
        assert_eq!(value["accessKey"], "AKIA_TEST");
        assert_eq!(value["secretKey"], "SECRET_TEST");
        assert!(value.get("access_key").is_none());
        assert!(value.get("secret_key").is_none());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub provider: CloudProvider,
    #[serde(default)]
    pub remote_path: String, // Base directory on remote
    #[serde(default)]
    pub sync_proxies: bool,
}

impl Default for CloudConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: CloudProvider::default(),
            remote_path: "/DIT_Pro_Sync".to_string(),
            sync_proxies: false,
        }
    }
}

/// A wrapper around OpenDAL Operator to provide consistent upload interface.
pub struct CloudClient {
    op: Operator,
}

impl CloudClient {
    /// Create a new client from config.
    pub fn new(config: &CloudProvider) -> Result<Self> {
        let op = match config {
            CloudProvider::S3 {
                endpoint,
                region,
                bucket,
                access_key,
                secret_key,
            } => {
                let builder = opendal::services::S3::default()
                    .endpoint(endpoint)
                    .region(region)
                    .bucket(bucket)
                    .access_key_id(access_key)
                    .secret_access_key(secret_key);

                Operator::new(builder)?.finish()
            }
            CloudProvider::WebDav {
                endpoint,
                username,
                password,
                root,
            } => {
                let builder = opendal::services::Webdav::default()
                    .endpoint(endpoint)
                    .username(username)
                    .password(password)
                    .root(root);

                Operator::new(builder)?.finish()
            }
        };

        Ok(Self { op })
    }

    /// Upload a local file to the remote storage in a streaming fashion (OOM-safe).
    pub async fn upload_file(&self, local_path: &Path, remote_path: &str) -> Result<()> {
        // Ensure parent directory exists on remote
        if let Some(parent) = Path::new(remote_path).parent() {
            let parent_str = parent.to_string_lossy().to_string();
            if !parent_str.is_empty() && parent_str != "/" {
                // Ensure parent ends with / for OpenDAL create_dir
                let dir_to_create = if parent_str.ends_with('/') {
                    parent_str
                } else {
                    format!("{}/", parent_str)
                };
                let _ = self.op.create_dir(&dir_to_create).await;
            }
        }

        // Open local file as a stream
        let mut file = fs::File::open(local_path)
            .await
            .with_context(|| format!("Failed to open local file {:?}", local_path))?;

        let metadata = file.metadata().await?;
        let file_size = metadata.len();

        // Use OpenDAL's writer for streaming upload
        let mut writer =
            self.op.writer(remote_path).await.with_context(|| {
                format!("Failed to initialize cloud writer for {}", remote_path)
            })?;

        use tokio::io::AsyncReadExt;
        let mut buffer = vec![0; 4 * 1024 * 1024]; // 4MB buffer
        loop {
            let bytes_read = file.read(&mut buffer).await?;
            if bytes_read == 0 {
                break;
            }
            writer.write(buffer[..bytes_read].to_vec()).await?;
        }

        writer
            .close()
            .await
            .with_context(|| format!("Failed to finalize upload for {}", remote_path))?;

        log::info!(
            "Successfully uploaded {} ({} bytes)",
            remote_path,
            file_size
        );
        Ok(())
    }

    /// Check connection/credentials by trying to list the root.
    pub async fn test_connection(&self) -> Result<()> {
        // Trying to list or stat root is more reliable than check() for some WebDAV providers
        self.op
            .list("/")
            .await
            .context("Cloud connection check failed (could not list root)")?;
        Ok(())
    }
}
