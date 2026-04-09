# Yundo

Yundo 是一个下载代理和下载历史看板项目。

- 后端：Rust + Axum
- 前端：React + Vite
- 存储：SQLite
- 部署：Rust 服务同时提供 API 和前端静态资源

## 功能

- 代理下载远程 `HTTP/HTTPS` 文件
- 支持 `HEAD` 探测和 `Range` 断点续传
- 本地磁盘缓存
- 下载历史记录与 7 天热度排序
- 基础 SSRF 拦截

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

常用路由：

- `/`
- `/healthz`
- `/api/proxy`
- `/api/recent`

## 服务参数

```bash
cargo run -- \
  --host 0.0.0.0 \
  --port 8080 \
  --cache-dir ./cache \
  --cache-size 1GiB \
  --frontend-dist ./frontend/dist
```

说明：

- `--cache-size` 为必填
- 支持纯字节和带单位写法，例如 `512MB`、`2GB`、`1GiB`
- 容器运行时也必须显式传入 `--cache-size`

## Docker 部署

仅保留 GitHub Container Registry 镜像部署方式。

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
  --cache-size 1GiB
```

访问地址：

```text
http://<server-ip>:8080
```

更新部署：

```bash
docker pull ghcr.io/veegn/yundo:latest
docker stop yundo
docker rm yundo
docker run -d \
  --name yundo \
  --restart unless-stopped \
  -p 8080:8080 \
  -v yundo-cache:/tmp/cache \
  ghcr.io/veegn/yundo:latest \
  --cache-size 1GiB
```

覆盖缓存大小示例：

```bash
docker run -d \
  --name yundo \
  --restart unless-stopped \
  -p 8080:8080 \
  -v yundo-cache:/tmp/cache \
  ghcr.io/veegn/yundo:latest \
  --cache-size 2GiB
```

## API

### `GET /api/proxy`

参数：

- `url`：目标下载地址，必填

示例：

```text
/api/proxy?url=https%3A%2F%2Fexample.com%2Ffile.zip
```

说明：

- 只允许 `http` 和 `https`
- 支持透传 `Range`
- `Range` 请求会绕过缓存
- 会尽量保持源文件名

### `GET /api/recent`

返回最近下载记录，最多 50 条。

主要字段：

- `url`
- `file_name`
- `file_size`
- `last_download_at`
- `count_7d`
- `score`

兼容路由：

- `GET /api/history`

## License

本项目使用 MIT License。见 [LICENSE](LICENSE)。
