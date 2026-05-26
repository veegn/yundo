use axum::http::StatusCode;
use reqwest::Client;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use tokio::net::TcpListener;

use precision_proxy::{
    app::build_router,
    common::{initialize_cache_dir, initialize_database, AppState},
    storage::LocalStorageBackend,
};
use std::path::PathBuf;
use std::sync::Arc;

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[tokio::test]
async fn test_distributed_storage_upload_lifecycle() {
    let cache_dir = tempfile::tempdir().unwrap();
    let app_addr = spawn_proxy_server_for_storage(cache_dir.path().to_path_buf()).await;
    let client = Client::new();

    // 1. Init Upload
    let init_res = client
        .post(format!("http://{app_addr}/api/uploads/init"))
        .header("content-type", "application/json")
        .body(serde_json::to_string(&serde_json::json!({
            "file_name": "test_distributed.txt",
            "file_size": 1024 * 1024 * 30, // 30MB
            "content_type": "text/plain",
            "replication_factor": 1
        })).unwrap())
        .send()
        .await
        .unwrap();

    assert_eq!(init_res.status(), StatusCode::OK);
    let init_text = init_res.text().await.unwrap();
    let init_body: Value = serde_json::from_str(&init_text).unwrap();
    let upload_id = init_body["upload_id"].as_str().unwrap().to_string();
    let chunk_size = init_body["chunk_size"].as_i64().unwrap();
    let total_chunks = init_body["total_chunks"].as_i64().unwrap();
    assert_eq!(total_chunks, 2); // 30MB / 16MB = 2 chunks

    // 2. Put chunk 0
    let chunk0_data = vec![b'A'; chunk_size as usize];
    let chunk0_sha256 = sha256_hex(&chunk0_data);
    let chunk0_res = client
        .put(format!("http://{app_addr}/api/uploads/{upload_id}/chunks/0"))
        .header("x-chunk-sha256", &chunk0_sha256)
        .body(chunk0_data)
        .send()
        .await
        .unwrap();
    assert_eq!(chunk0_res.status(), StatusCode::OK);

    // 3. Put chunk 1 (remaining bytes)
    let chunk1_size = (1024 * 1024 * 30) - chunk_size;
    let chunk1_data = vec![b'B'; chunk1_size as usize];
    let chunk1_sha256 = sha256_hex(&chunk1_data);
    let chunk1_res = client
        .put(format!("http://{app_addr}/api/uploads/{upload_id}/chunks/1"))
        .header("x-chunk-sha256", &chunk1_sha256)
        .body(chunk1_data)
        .send()
        .await
        .unwrap();
    assert_eq!(chunk1_res.status(), StatusCode::OK);

    // 4. Complete upload
    let complete_res = client
        .post(format!("http://{app_addr}/api/uploads/{upload_id}/complete"))
        .send()
        .await
        .unwrap();
    assert_eq!(complete_res.status(), StatusCode::OK);
    let complete_text = complete_res.text().await.unwrap();
    let complete_body: Value = serde_json::from_str(&complete_text).unwrap();
    let file_id = complete_body["file_id"].as_str().unwrap().to_string();

    // 5. Download the file via distributed chunk routing
    let download_res = client
        .get(format!("http://{app_addr}/api/filebox/download/{file_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(download_res.status(), StatusCode::OK);

    // Validate stream
    let downloaded_bytes = download_res.bytes().await.unwrap();
    assert_eq!(downloaded_bytes.len(), 1024 * 1024 * 30);
    assert_eq!(&downloaded_bytes[0..10], &[b'A'; 10]);
    assert_eq!(&downloaded_bytes[downloaded_bytes.len() - 10..], &[b'B'; 10]);

    // 6. Delete file
    let delete_res = client
        .delete(format!("http://{app_addr}/api/filebox/delete/{file_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_res.status(), StatusCode::OK);

    let list_res = client
        .get(format!("http://{app_addr}/api/filebox/files"))
        .send()
        .await
        .unwrap();
    assert_eq!(list_res.status(), StatusCode::OK);
    let list_text = list_res.text().await.unwrap();
    let list_body: Value = serde_json::from_str(&list_text).unwrap();
    assert!(list_body["files"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_distributed_storage_sha256_mismatch() {
    let cache_dir = tempfile::tempdir().unwrap();
    let app_addr = spawn_proxy_server_for_storage(cache_dir.path().to_path_buf()).await;
    let client = Client::new();

    let init_res = client
        .post(format!("http://{app_addr}/api/uploads/init"))
        .header("content-type", "application/json")
        .body(serde_json::to_string(&serde_json::json!({
            "file_name": "test_mismatch.txt",
            "file_size": 1024,
            "replication_factor": 1
        })).unwrap())
        .send()
        .await
        .unwrap();
    let init_text = init_res.text().await.unwrap();
    let init_body: Value = serde_json::from_str(&init_text).unwrap();
    let upload_id = init_body["upload_id"].as_str().unwrap().to_string();

    let chunk_data = vec![b'A'; 1024];
    let fake_sha256 = sha256_hex(b"FAKE_DATA");

    let chunk_res = client
        .put(format!("http://{app_addr}/api/uploads/{upload_id}/chunks/0"))
        .header("x-chunk-sha256", &fake_sha256)
        .body(chunk_data)
        .send()
        .await
        .unwrap();

    assert_eq!(chunk_res.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let status_res = client
        .get(format!("http://{app_addr}/api/uploads/{upload_id}/status"))
        .send()
        .await
        .unwrap();
    let status_text = status_res.text().await.unwrap();
    let status_body: Value = serde_json::from_str(&status_text).unwrap();
    assert!(status_body["failed_chunks"].as_array().unwrap().contains(&serde_json::json!(0)));
}

async fn spawn_proxy_server_for_storage(cache_dir: PathBuf) -> SocketAddr {
    initialize_cache_dir(&cache_dir).await;
    let db = initialize_database(&cache_dir).await;
    let storage_backend = std::sync::Arc::new(LocalStorageBackend::new(
        cache_dir.join("storage"),
    ));
    let state = Arc::new(AppState {
        client: Client::new(),
        cache_dir,
        max_cache_size: 100 * 1024 * 1024,
        filebox_size: 100 * 1024 * 1024,
        db,
        frontend_dist: PathBuf::from("./frontend/missing-dist"),
        base_path: "/".to_string(),
        storage_backend,
        node_mode: precision_proxy::config::NodeMode::All,
        node_config: precision_proxy::common::NodeConfig {
            node_id: "local".to_string(),
            endpoint: None,
            zone: None,
            api_endpoint: None,
            heartbeat_interval_secs: 30,
            heartbeat_ttl_secs: 90,
            default_chunk_size: 16 * 1024 * 1024,
            default_replication_factor: 1,
        },
        internal_token: None,
    });

    let router = build_router(state, PathBuf::from("./frontend/missing-dist"));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    addr
}
