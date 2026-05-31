# Yundo

Yundo 是一个全能型下载代理、Web 代理及文件中转平台。

- 后端：Rust + Axum
- 前端：React + Vite
- 存储：SQLite
- 部署：Rust 服务同时提供 API 和前端静态资源

## 功能

- **代理下载**：高效代理远程 `HTTP/HTTPS` 文件，支持 `HEAD` 探测。
- **断点续传**：支持 `Range` 请求，完美适配各类下载工具。
- **本地缓存**：智能磁盘缓存机制，支持自动清理和空间配额管理。
- **Web 代理**：内置网页浏览器代理，支持 HTML 链接重写及 Cookie 会话隔离。
- **FileBox**：便捷的文件上传与分享功能，支持设置有效期。
- **监控告警**：内置 Prometheus 指标采集（`/metrics`）及健康检查。
- **安全防护**：深度 SSRF 拦截、主机名过滤及 API Key 认证。
- **性能卓越**：基于 Rust 异步 IO，支持高并发处理及优雅停机。

## 本地运行

环境要求：

- Node.js 18+
- npm
- Rust stable

安装依赖：

```bash
npm install
cargo build
```

前端开发：

```bash
npm run dev --workspace=frontend
```

完整运行：

```bash
npm run build
cargo run -- --cache-size 1GiB
```

默认访问地址：

```text
http://127.0.0.1:8080
```

## 服务参数

```bash
cargo run -- \
  --host 0.0.0.0 \
  --port 8080 \
  --cache-dir ./cache \
  --cache-size 1GiB \
  --max-file-size 500MB \
  --filebox-size 5GB \
  --api-key your-secret-key \
  --base-path / \
  --frontend-dist ./frontend/dist
```

参数说明：

| 参数 | 描述 | 默认值 |
| :--- | :--- | :--- |
| `--cache-size` | **(必填)** 下载代理的最大缓存容量 | - |
| `--cache-dir` | 数据存储目录（缓存、数据库、文件箱） | `./cache` |
| `--host` | 绑定主机地址 | `0.0.0.0` |
| `--port` | 监听端口 | `8080` |
| `--max-file-size` | 下载代理允许的最大文件大小 | `500MB` |
| `--filebox-size` | 文件箱总容量上限 | `5GB` |
| `--api-key` | 启用 API Key 认证（Header: `X-API-Key`） | 无 |
| `--rate-limit-per-minute` | 每分钟请求速率限制 | `60` |
| `--base-path` | 服务挂载的基础路径 | `/` |
| `--frontend-dist` | 前端静态资源目录 | `./frontend/dist` |

## Docker 部署

拉取镜像：

```bash
docker pull ghcr.io/veegn/yundo:latest
```

启动容器：

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

## API 指南

### 1. 下载代理 (`GET /api/proxy`)
- **参数**: `url` (必填, URL 编码)
- **支持**: `Range` 请求透传、文件名自动提取、SSRF 保护。

### 2. 网页浏览 (`GET /browse`)
- **参数**: `url` (必填)
- **功能**: 在代理环境下浏览网页，自动重写静态资源和超链接路径。

### 3. 文件箱 (`FileBox`)
- **上传**: `POST /api/filebox/upload` (支持大文件分片上传)
- **远程上传**: `POST /api/filebox/remote-upload` (通过 URL 异步转存)
- **下载**: `GET /api/filebox/download/:id`
- **列表**: `GET /api/filebox/files`
- **删除**: `DELETE /api/filebox/delete/:id`

### 4. 历史记录 (`GET /api/recent`)
- 返回最近 50 条下载记录，包含热度得分（基于 7 天下载频次）。

### 5. 系统监控
- **健康检查**: `GET /healthz`
- **指标采集**: `GET /metrics` (Prometheus 格式)

## 安全说明

- **SSRF 拦截**: 自动屏蔽私有网段（127.0.0.1, 192.168.x.x 等）及受限制的主机名。
- **认证**: 若配置了 `--api-key`，所有 API 请求需在 Header 中携带 `X-API-Key`。

## License

本项目使用 MIT License。见 [LICENSE](LICENSE)。
