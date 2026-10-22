# Yundo

Yundo 是一个自托管的 HTTP/HTTPS 下载代理和临时文件箱。Rust 服务同时提供 API 与构建后的 React 前端，数据保存在本地磁盘和 SQLite 中。

## 当前功能

- 下载代理：支持 `GET`、`HEAD`、`Range`，会提取文件名并转发常用响应头。
- 代理缓存：完整下载会写入磁盘缓存；带 `Range` 的代理请求直接转发，不写入缓存。
- 临时文件箱：支持普通 multipart 上传、分块上传、通过 URL 转存、下载和删除；文件默认保留 7 天。
- 下载历史：记录代理下载，`/api/recent` 返回按近 7 天下载次数和最近使用时间排序的最多 50 条记录。
- 可观测性：提供 `/healthz` 健康检查和 Prometheus 格式的 `/metrics` 指标。
- 安全控制：可选 API Key 认证；代理和远程转存检查初始 URL、重定向目标及实际连接使用的 DNS 地址，拒绝本机、私有网段和链路本地目标。

## 运行要求

- Node.js 18+
- npm
- Rust stable

安装依赖：

```bash
npm install
cargo build
```

前端开发服务器：

```bash
npm run dev
```

启动后端（`--cache-size` 必须提供）：

```bash
cargo run -- --cache-size 1GiB
```

构建前端并运行完整服务：

```bash
npm run build
cargo run -- --cache-size 1GiB
```

默认地址为 `http://127.0.0.1:8080`。如果 `frontend/dist/index.html` 不存在，服务仍会启动，但只提供 API。

## 配置参数

```bash
cargo run -- \
  --host 0.0.0.0 \
  --port 8080 \
  --cache-dir ./cache \
  --cache-size 1GiB \
  --max-file-size 0 \
  --api-key your-secret-key \
  --base-path / \
  --frontend-dist ./frontend/dist
```

| 参数 | 默认值 | 说明 |
| :--- | :--- | :--- |
| `--cache-dir` | `./cache` | 缓存目录，同时保存 SQLite 数据库、代理缓存和 FileBox 文件。 |
| `--cache-size` | 必填 | 代理缓存、磁盘上的 FileBox 文件和临时分块共同使用的总空间上限；支持字节、`KB`/`MB`/`GB`/`TB` 及 `KiB`/`MiB`/`GiB`/`TiB`。 |
| `--max-file-size` | `0` | 单文件上传大小限制；`0` 表示不限制。普通上传、远程转存、分块接收和合并后的总大小均检查。 |
| `--host` | `0.0.0.0` | 监听地址，必须是 IP 地址。 |
| `--port` | `8080` | 监听端口。 |
| `--frontend-dist` | `./frontend/dist` | 前端构建产物目录。 |
| `--base-path` | `/` | 挂载基础路径，例如 `/yundo`。 |
| `--api-key` | 未启用 | 启用后保护 FileBox 的上传、分块、转存、中止和删除接口。 |
| `--filebox-size` | `5GB` | CLI 兼容参数；当前没有独立 FileBox 配额，实际空间限制由 `--cache-size` 控制。 |

缓存目录主要包含：

- `proxy.db`：FileBox 元数据、下载历史和下载事件。
- `*.data` / `*.meta`：代理缓存内容及响应元数据。
- `filebox/`：有效 FileBox 文件。
- `filebox_tmp/`：分块上传中的临时文件。

配额按文件内容的实际字节数计算，包括代理缓存、FileBox 文件、临时分块和正在写入的内容；不包含 SQLite 与响应元数据。写入前会预留空间，失败、中止及删除后释放。分块合并期间需要同时容纳原始分块和合并结果，请预留约一份文件大小的额外空间。代理缓存空间不足时会继续转发下载，放弃该次缓存。

## API

以下路径默认挂载在根路径；使用 `--base-path /yundo` 或反向代理提供 `X-Forwarded-Prefix: /yundo` 时，需要在路径前加对应前缀。

### 下载代理

```text
GET  /api/proxy?url=<URL 编码后的 HTTP/HTTPS 地址>
HEAD /api/proxy?url=<URL 编码后的 HTTP/HTTPS 地址>
```

代理会透传 `Range` 请求和部分下载相关响应头，并根据响应或 URL 推断文件名。完整成功响应会缓存；带 `Range` 的请求不缓存。

示例：

```bash
curl -L "http://127.0.0.1:8080/api/proxy?url=https%3A%2F%2Fexample.com%2Ffile.zip" -o file.zip
```

### FileBox

| 方法 | 路径 | 说明 |
| :--- | :--- | :--- |
| `GET` | `/api/filebox/files` | 返回空间使用量和未过期文件列表。 |
| `POST` | `/api/filebox/upload` | multipart 上传一个或多个文件。字段名可为任意文件字段。 |
| `POST` | `/api/filebox/upload-chunk` | multipart 上传分块，字段为 `upload_id`、`chunk_index`、`file`。 |
| `POST` | `/api/filebox/upload-complete` | JSON 合并分块：`{"upload_id":"...","file_name":"...","total_chunks":3}`。 |
| `POST` | `/api/filebox/upload-abort` | JSON 取消分块上传：`{"upload_id":"..."}`。 |
| `POST` | `/api/filebox/remote-upload` | JSON 远程转存：`{"url":"https://example.com/file.zip"}`。 |
| `GET` | `/api/filebox/download/:id` | 下载文件，支持 `Range`。 |
| `DELETE` | `/api/filebox/delete/:id` | 删除文件。 |

FileBox 下载链接不需要额外签名；文件过期或记录不存在时返回 `404`。启用 API Key 后，上传、分块、转存、中止和删除请求需要携带以下任一请求头：

```text
Authorization: Bearer your-secret-key
X-API-Key: your-secret-key
```

启用认证后，文件箱页面会显示访问密钥输入框；密钥保存在当前浏览器会话中，上传、合并、中止、转存和删除时自动携带。文件列表、下载代理、下载链接、历史记录和指标仍为公开接口。

分块接收按流写入磁盘，单块最多 72 MiB。同一 `upload_id` 与 `chunk_index` 重复提交返回 `409`，原有分块保持不变。JSON 请求体限制为 64 KiB。

上游请求直接连接经过检查的地址，不使用 `HTTP_PROXY` / `HTTPS_PROXY` 等环境代理，以免代理绕过连接地址检查。

### 历史、健康检查和指标

```text
GET /api/recent
GET /healthz
GET /metrics
```

`/api/recent` 最多返回 50 条记录，字段包括 `slug`、`url`、`file_name`、`file_size`、`last_download_at`、`count_7d` 和 `score`。`/healthz` 成功时返回 `ok`。Prometheus 记录器安装失败时，`/metrics` 返回 `503`。

## Docker

直接运行镜像时必须提供 `--cache-size`：

```bash
docker run -d \
  --name yundo \
  --restart unless-stopped \
  -p 8080:8080 \
  -v yundo-cache:/tmp/cache \
  ghcr.io/veegn/yundo:latest \
  --cache-size 1GiB \
  --api-key my-secret-auth
```

## 检查与测试

```bash
npm run lint
npm run build
cargo fmt --check
cargo test
```

## License

本项目使用 MIT License，详见 [LICENSE](LICENSE)。
