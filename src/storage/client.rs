use super::backend::{ChunkWriteResult, VerifyResult};
use bytes::Bytes;
use reqwest::Client;
use std::time::Duration;

/// HTTP client for communicating with remote Storage Node internal APIs.
#[derive(Clone)]
pub struct StorageNodeClient {
    client: Client,
}

impl StorageNodeClient {
    pub fn new() -> Self {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(180))
            .build()
            .expect("failed to build storage node HTTP client");
        Self { client }
    }

    /// Write a chunk to a remote storage node.
    #[allow(clippy::too_many_arguments)]
    pub async fn put_chunk(
        &self,
        endpoint: &str,
        object_key: &str,
        data: Bytes,
        expected_sha256: &str,
        token: &str,
        file_id: &str,
        chunk_index: i64,
    ) -> Result<ChunkWriteResult, StorageClientError> {
        let url = format!(
            "{}/internal/chunks?object_key={}",
            endpoint.trim_end_matches('/'),
            urlencoding::encode(object_key)
        );

        let response = self
            .client
            .put(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("X-Chunk-Sha256", expected_sha256)
            .header("X-File-Id", file_id)
            .header("X-Chunk-Index", chunk_index.to_string())
            .header("Content-Length", data.len().to_string())
            .body(data)
            .send()
            .await
            .map_err(|e| StorageClientError::Network(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(StorageClientError::Remote(status, body));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| StorageClientError::Network(e.to_string()))?;

        Ok(ChunkWriteResult {
            size_bytes: body["size_bytes"].as_u64().unwrap_or(0),
            sha256: body["sha256"].as_str().unwrap_or("").to_string(),
        })
    }

    /// Read a chunk from a remote storage node as bytes.
    pub async fn get_chunk(
        &self,
        endpoint: &str,
        object_key: &str,
        token: &str,
    ) -> Result<Bytes, StorageClientError> {
        let url = format!(
            "{}/internal/chunks?object_key={}",
            endpoint.trim_end_matches('/'),
            urlencoding::encode(object_key)
        );

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| StorageClientError::Network(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(StorageClientError::Remote(status, body));
        }

        response
            .bytes()
            .await
            .map_err(|e| StorageClientError::Network(e.to_string()))
    }

    /// Delete a chunk from a remote storage node (idempotent).
    pub async fn delete_chunk(
        &self,
        endpoint: &str,
        object_key: &str,
        token: &str,
    ) -> Result<(), StorageClientError> {
        let url = format!(
            "{}/internal/chunks?object_key={}",
            endpoint.trim_end_matches('/'),
            urlencoding::encode(object_key)
        );

        let response = self
            .client
            .delete(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| StorageClientError::Network(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(StorageClientError::Remote(status, body));
        }

        Ok(())
    }

    /// Verify a chunk on a remote storage node.
    pub async fn verify_chunk(
        &self,
        endpoint: &str,
        object_key: &str,
        expected_sha256: &str,
        token: &str,
    ) -> Result<VerifyResult, StorageClientError> {
        let url = format!(
            "{}/internal/chunks/verify",
            endpoint.trim_end_matches('/')
        );

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({
                "object_key": object_key,
                "expected_sha256": expected_sha256,
            }))
            .send()
            .await
            .map_err(|e| StorageClientError::Network(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(StorageClientError::Remote(status, body));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| StorageClientError::Network(e.to_string()))?;

        Ok(VerifyResult {
            exists: body["exists"].as_bool().unwrap_or(false),
            valid: body["valid"].as_bool().unwrap_or(false),
            size_bytes: body["size_bytes"].as_u64().unwrap_or(0),
            sha256: body["sha256"].as_str().map(|s| s.to_string()),
        })
    }
}

impl Default for StorageNodeClient {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum StorageClientError {
    Network(String),
    Remote(u16, String),
}

impl std::fmt::Display for StorageClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageClientError::Network(msg) => write!(f, "storage client network error: {msg}"),
            StorageClientError::Remote(status, body) => {
                write!(f, "storage node returned {status}: {body}")
            }
        }
    }
}

impl std::error::Error for StorageClientError {}
