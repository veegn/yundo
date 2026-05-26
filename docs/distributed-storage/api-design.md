# 分布式存储 API 设计

本文定义上传、下载和 Storage Node 内部接口。API 目标是支持 chunk 级恢复、幂等上传、下载副本切换和安全 object key 访问。

## 1. 上传会话

### 1.1 创建上传会话

```text
POST /api/uploads/init
```

请求：

```json
{
  "file_name": "example.zip",
  "file_size": 1073741824,
  "content_type": "application/zip",
  "replication_factor": 2,
  "file_sha256": "optional-full-file-sha256"
}
```

响应：

```json
{
  "upload_id": "upl_xxx",
  "file_id": "file_xxx",
  "chunk_size": 16777216,
  "total_chunks": 64,
  "concurrency_hint": 2,
  "chunk_upload_url": "/api/uploads/upl_xxx/chunks/{index}",
  "expires_at": "2026-05-25T14:30:00Z"
}
```

规则：

- `chunk_size` 在上传会话创建时确定，并写入元数据。
- `content_type` 可选。客户端可提交，未提交时服务端根据 `file_name` 后缀推断。
- 客户端可以提交 `file_sha256`，但服务端第一阶段至少强制校验 chunk sha256。
- 控制平面可以根据文件大小、网络质量和反向代理请求体限制决定 chunk size。
- 第一阶段不做跨文件 chunk 去重，每次上传独立存储。

### 1.2 查询上传状态

```text
GET /api/uploads/:upload_id/status
```

响应：

```json
{
  "status": "active",
  "chunk_size": 16777216,
  "total_chunks": 64,
  "uploaded_chunks": [0, 1, 2, 5],
  "missing_chunks": [3, 4, 6, 7],
  "failed_chunks": [8],
  "conflict_chunks": []
}
```

该接口基于 `upload_session_chunks` 表返回状态，用于断点恢复。

### 1.3 上传单个 chunk

```text
PUT /api/uploads/:upload_id/chunks/:index
```

请求头：

```text
Content-Length: 16777216
X-Chunk-Sha256: <chunk-sha256>
```

规则：

- `:index` 必须在 `[0, total_chunks)` 范围内。
- 非最后一个 chunk 的大小必须等于会话 `chunk_size`。
- 最后一个 chunk 可小于 `chunk_size`。
- 服务端写入 StorageBackend 或 Storage Node 后必须校验 sha256。
- 同一 chunk 重传时，sha256 相同可直接返回成功。
- 同一 chunk 重传时，sha256 不同必须拒绝并记录 `conflict`。

响应：

```json
{
  "success": true,
  "upload_id": "upl_xxx",
  "chunk_index": 3,
  "sha256": "...",
  "status": "uploaded"
}
```

### 1.4 完成上传

```text
POST /api/uploads/:upload_id/complete
```

完成流程：

```text
load upload session
  -> verify all chunk indexes uploaded
  -> create file_chunks
  -> create initial chunk_replicas
  -> if every chunk has at least one ready replica:
       file.status = partial_ready
     else:
       file.status = failed or uploading
  -> enqueue repair tasks for insufficient replicas
  -> upload_session.status = completed
```

complete 操作必须在单个数据库事务中完成。如果中途失败（如创建了 file_chunks 但 chunk_replicas 写入失败），必须回滚以避免元数据不一致。PostgreSQL 模式下可配合 advisory lock 防止并发 complete。

file 的 `content_type` 从 upload session 中继承（客户端可在 init 时提交），或由服务端根据 `file_name` 后缀推断。

响应：

```json
{
  "success": true,
  "file_id": "file_xxx",
  "status": "partial_ready",
  "download_url": "/api/filebox/download/file_xxx"
}
```

`partial_ready` 表示文件允许下载，但副本数尚未达到目标，后台 repair worker 会继续补齐。

### 1.5 取消上传

```text
POST /api/uploads/:upload_id/abort
```

取消流程：

```text
mark upload_session = aborted
  -> mark uploaded temporary chunks deleting
  -> enqueue storage_gc_tasks
  -> return success
```

删除必须异步补偿。即使某些 Storage Node 暂时不可达，也不能阻塞 abort 返回。

## 2. 文件下载

保持现有下载入口兼容：

```text
GET /api/filebox/download/:id
```

内部流程：

```text
query new files table
  -> if found:
       query chunks
       select best ready replica for each chunk
       stream chunks in order
       failed chunk switches to another replica
     else:
       fallback to legacy cache/filebox/{id}
```

下载时可读取 `partial_ready` 和 `ready` 文件：

- `partial_ready`：允许下载，只要每个 chunk 至少有一个 ready 副本。
- `ready`：允许下载，且达到目标副本数。
- `repair_needed`：如果每个 chunk 仍有至少一个 ready 副本，可以允许下载，但应记录降级事件。
- `failed`、`deleting`、`deleted`：不允许下载。

### 2.1 下载错误模型

流式输出中不可恢复的场景：

- HTTP 响应已 committed 200 状态码后，某个 chunk 所有副本失败时，只能截断连接。客户端通过 `Content-Length` 与实际接收字节数不符来检测不完整下载。
- `files.file_size` 已知时，应在响应头中返回 `Content-Length`。

Range 请求支持：

- chunk 化后 Range 请求需要计算跨 chunk 偏移，复杂度较高。
- 第一阶段不强制支持 Range，但应预留接口设计空间。
- 后续可通过下载计划接口支持客户端级 Range 下载。

## 3. 下载计划接口

后续可以增加高级下载计划接口，用于客户端直连 Storage Node 或并行下载。

```text
GET /api/files/:file_id/download-plan
```

响应：

```json
{
  "file_id": "file_xxx",
  "chunk_size": 16777216,
  "file_size": 1073741824,
  "file_sha256": "optional-full-file-sha256",
  "chunks": [
    {
      "index": 0,
      "size_bytes": 16777216,
      "sha256": "...",
      "replicas": [
        {
          "node_id": "node-a",
          "url": "https://api.example.com/api/files/file_xxx/chunks/0?replica=node-a&token=...",
          "expires_at": "2026-05-24T15:00:00Z"
        }
      ]
    }
  ]
}
```

如果暴露下载计划，URL 必须短有效期签名，且不能泄露未授权 object key。

## 4. Storage Node 内部接口

### 4.1 object key 传递原则

不建议使用：

```text
PUT /internal/chunks/:object_key
```

因为 object key 可能包含 `/`，例如：

```text
files/{file_id}/chunks/{chunk_index}
```

直接作为路径参数容易产生路由歧义、编码问题和路径穿越风险。

推荐使用 query、请求头或 JSON body 传递 object key：

```text
PUT /internal/chunks?object_key=<url-encoded-key>
GET /internal/chunks?object_key=<url-encoded-key>
DELETE /internal/chunks?object_key=<url-encoded-key>
POST /internal/chunks/verify
```

或者：

```text
X-Object-Key: files/file_xxx/chunks/000001
```

Storage Node 必须对 object key 做规范化校验，不能直接拼接用户输入路径。

### 4.2 写入 chunk

```text
PUT /internal/chunks?object_key=<encoded-object-key>
```

请求头：

```text
Authorization: Bearer <internal-token>
Content-Length: 16777216
X-Chunk-Sha256: <chunk-sha256>
X-File-Id: file_xxx
X-Chunk-Index: 3
```

处理规则：

```text
validate internal auth
  -> validate object_key format
  -> write to temporary path
  -> fsync or equivalent durability step if configured
  -> compute sha256
  -> compare X-Chunk-Sha256
  -> atomic rename to final path
  -> return ready
```

响应：

```json
{
  "success": true,
  "object_key": "files/file_xxx/chunks/000003",
  "size_bytes": 16777216,
  "sha256": "..."
}
```

### 4.3 读取 chunk

```text
GET /internal/chunks?object_key=<encoded-object-key>
```

处理规则：

- 校验内部认证。
- 校验 object key 格式。
- 只允许读取 Storage Node 管理目录内的对象。
- 返回 `application/octet-stream`。
- 可选返回 `X-Chunk-Sha256`。

### 4.4 删除 chunk

```text
DELETE /internal/chunks?object_key=<encoded-object-key>
```

删除规则：

- 删除必须幂等。
- object 不存在时返回成功。
- 删除失败时由控制平面保留 `storage_gc_tasks` 后续重试。

### 4.5 校验 chunk

```text
POST /internal/chunks/verify
```

请求：

```json
{
  "object_key": "files/file_xxx/chunks/000003",
  "expected_sha256": "..."
}
```

响应：

```json
{
  "exists": true,
  "valid": true,
  "size_bytes": 16777216,
  "sha256": "..."
}
```

## 5. 节点心跳接口

```text
POST /api/storage/nodes/:id/heartbeat
```

请求：

```json
{
  "capacity_bytes": 1099511627776,
  "used_bytes": 123456789,
  "active_uploads": 4,
  "active_downloads": 12,
  "active_replications": 2,
  "disk_ok": true,
  "avg_rtt_ms": 180,
  "p95_rtt_ms": 420,
  "packet_loss": 0.08,
  "timeout_rate": 0.03
}
```

心跳只上报观测指标，最终调度状态由控制平面计算。

## 6. 错误码建议

| 场景 | HTTP 状态 | 说明 |
| --- | --- | --- |
| 上传会话不存在或过期 | 404 / 410 | 客户端应重新 init |
| chunk index 越界 | 400 | 请求非法 |
| chunk sha256 不匹配 | 422 | 内容损坏或传输错误 |
| chunk sha256 冲突 | 409 | 同 index 重传内容不一致 |
| 存储节点无容量 | 507 | 可重新调度其他节点 |
| 存储节点不可达 | 503 | 可重试或切换节点 |
| 内部认证失败 | 401 / 403 | 拒绝访问 |
