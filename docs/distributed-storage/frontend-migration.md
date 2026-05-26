# 前端改造设计

本文定义 FileBox 前端从当前分片上传方式迁移到新 chunk 存储 API 的变更内容。

## 1. 当前前端实现

当前 `frontend/src/pages/FileBox.tsx` 的上传流程：

```text
client generates upload_id (Math.random + Date.now)
  -> slice file into 72MB chunks
  -> POST /api/filebox/upload-chunk (multipart: upload_id, chunk_index, file)
  -> POST /api/filebox/upload-complete (JSON: upload_id, file_name, total_chunks)
```

主要问题：

- `upload_id` 由客户端生成，无服务端校验。
- chunk size 硬编码为 72MB，超过设计方案的 max_chunk_size=64MiB 上限。
- 没有 chunk sha256 校验。
- 没有上传状态查询和断点恢复。
- 并发数硬编码为 3，不受服务端控制。

## 2. 新上传流程

```text
POST /api/uploads/init
  -> receive upload_id, chunk_size, total_chunks, concurrency_hint
  -> slice file into server-specified chunk_size
  -> for each chunk:
       compute sha256 via Web Crypto API
       PUT /api/uploads/:upload_id/chunks/:index
         headers: X-Chunk-Sha256, Content-Length
  -> POST /api/uploads/:upload_id/complete
```

## 3. 关键变更

### 3.1 upload_id 从服务端获取

移除客户端 `Math.random()` 生成逻辑，改为从 init 响应中获取。

### 3.2 chunk size 动态化

移除硬编码的 `CHUNK_SIZE = 72 * 1024 * 1024`，改为从 init 响应的 `chunk_size` 字段获取。

### 3.3 chunk sha256 计算

使用 Web Crypto API 计算每个 chunk 的 sha256：

```javascript
async function computeSha256(chunk) {
  const buffer = await chunk.arrayBuffer();
  const hashBuffer = await crypto.subtle.digest('SHA-256', buffer);
  const hashArray = Array.from(new Uint8Array(hashBuffer));
  return hashArray.map(b => b.toString(16).padStart(2, '0')).join('');
}
```

注意事项：

- `crypto.subtle` 仅在 HTTPS 或 localhost 下可用。
- 大 chunk 计算 sha256 可能阻塞 UI，建议使用 Web Worker。
- sha256 计算和上传可以流水线化：计算下一个 chunk 的 sha256 同时上传当前 chunk。

### 3.4 并发数动态化

初始并发数从 init 响应的 `concurrency_hint` 获取，运行中根据成功率动态调整。

建议客户端实现简化版动态并发：

```text
initial concurrency = concurrency_hint (default 2)
consecutive 8 success -> concurrency + 1, max 6
2 consecutive timeout or error -> concurrency / 2, min 1
```

### 3.5 上传请求格式变更

当前使用 multipart/form-data 上传 chunk，新方案使用 PUT 请求直接发送二进制 body：

```text
PUT /api/uploads/:upload_id/chunks/:index
Content-Length: <chunk-size>
X-Chunk-Sha256: <sha256-hex>

<binary chunk body>
```

这比 multipart 编码更高效，避免了 FormData 额外开销。前端需要将 `XMLHttpRequest` 的发送方式从 `formData.append` 改为直接发送 `Blob`。

### 3.6 上传状态查询与断点恢复

新增断点恢复能力：

```text
on upload start:
  try GET /api/uploads/:upload_id/status
  if active session found:
    skip already uploaded chunks
    resume from missing chunks
  else:
    POST /api/uploads/init to create new session
```

断点恢复 UI 建议：

- 上传中断后，在文件列表中显示"未完成上传"条目。
- 用户可以选择"继续上传"或"取消上传"。
- 上传 ID 可存储在 localStorage 中，页面刷新后可恢复。

### 3.7 abort 接口变更

```text
当前: POST /api/filebox/upload-abort (JSON: { upload_id })
新版: POST /api/uploads/:upload_id/abort
```

## 4. 错误处理变更

| 场景 | 当前行为 | 新行为 |
| --- | --- | --- |
| chunk 上传失败 | 取消所有 XHR，abort 整个上传 | 重试该 chunk（最多 5 次），指数退避 |
| 网络断开 | 上传失败，需重新上传整个文件 | 页面恢复后查询状态，只补传缺失 chunk |
| sha256 不匹配 | 无校验 | 服务端返回 422，客户端重新读取该 chunk 并重传 |
| sha256 冲突 | 无校验 | 服务端返回 409，客户端提示错误 |
| 会话过期 | 无会话概念 | 服务端返回 410，客户端需重新 init |
| 空间不足 | 整个上传失败 | init 阶段可预检，chunk 上传时返回 507 |

## 5. 下载兼容

下载路径保持不变：

```text
GET /api/filebox/download/:id
```

服务端内部自动判断文件是旧模式还是新 chunk 模式，前端无需修改下载逻辑。

## 6. 整文件 sha256（可选）

前端可以在上传前或上传过程中使用 Web Crypto API 计算整文件 sha256，在 init 请求中提交 `file_sha256`。

由于大文件整文件 sha256 计算可能需要较长时间，建议：

- 小文件（<100MB）：在 init 前计算完成。
- 大文件（>=100MB）：边上传 chunk 边在 Web Worker 中流式计算，complete 时提交。

## 7. 迁移步骤

1. 阶段一期间，新 API 与旧 API 并存。前端先切换到新 API。
2. 旧上传接口（`/api/filebox/upload-chunk`、`/api/filebox/upload-complete`）保留但标记为 deprecated。
3. 前端移除旧上传逻辑后，后端可在后续版本删除旧接口。

## 8. Web Worker 建议

sha256 计算属于 CPU 密集操作，建议在 Web Worker 中执行：

```text
main thread                    worker thread
  |                              |
  |-- postMessage(chunk) ------->|
  |                              |-- compute sha256
  |<-- postMessage(sha256) ------|
  |
  |-- PUT chunk with sha256
```

这保证 sha256 计算不阻塞上传进度条和 UI 交互。
