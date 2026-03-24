# Yundo

Yundo 是一个下载代理和下载历史看板项目。

- 后端：Rust + Axum
- 前端：React + Vite
- 存储：SQLite
- 部署：Rust 服务同时提供 API 和前端静态资源

## 功能

- 代理下载远程 `HTTP/HTTPS` 文件
- 记录最近下载历史
- 本地磁盘缓存
- 7 天下载热度统计
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

开发前端：

```bash
npm run dev --workspace=frontend
```

完整运行：

```bash
npm run build
cargo run -- --cache-size 1073741824
```

默认地址：

```text
http://127.0.0.1:8080
```

常用路径：

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
  --cache-size 1073741824 \
  --frontend-dist ./frontend/dist
```

说明：

- `--cache-size` 为必填，单位字节
- 默认 Docker 镜像内置 `1 GiB` 缓存上限

## Docker 部署

仓库已提供：

- [Dockerfile](/D:/dev/yundo/Dockerfile)
- [.dockerignore](/D:/dev/yundo/.dockerignore)
- [docker-image.yml](/D:/dev/yundo/.github/workflows/docker-image.yml)

镜像特点：

- 多阶段构建
- Rust release 构建
- distroless 运行层
- SQLite bundled 静态构建

本地构建镜像：

```bash
docker build -t yundo:latest .
```

本地运行镜像：

```bash
docker run --rm -p 8080:8080 yundo:latest
```

持久化缓存和数据库：

```bash
docker run --rm -p 8080:8080 -v yundo-cache:/tmp/cache yundo:latest
```

## 通过 GitHub Docker 镜像部署

GitHub Actions 会自动构建并推送镜像到 GHCR：

```text
ghcr.io/<owner>/<repo>:<tag>
```

触发方式：

- push 到 `master`
- push tag，例如 `v1.0.0`
- pull request
- 手动触发 workflow

拉取镜像：

```bash
docker pull ghcr.io/<owner>/<repo>:latest
```

如果镜像为私有，需要先登录：

```bash
echo <GITHUB_TOKEN> | docker login ghcr.io -u <GITHUB_USERNAME> --password-stdin
```

服务器部署：

```bash
docker run -d \
  --name yundo \
  --restart unless-stopped \
  -p 8080:8080 \
  -v yundo-cache:/tmp/cache \
  ghcr.io/<owner>/<repo>:latest
```

更新部署：

```bash
docker pull ghcr.io/<owner>/<repo>:latest
docker stop yundo
docker rm yundo
docker run -d \
  --name yundo \
  --restart unless-stopped \
  -p 8080:8080 \
  -v yundo-cache:/tmp/cache \
  ghcr.io/<owner>/<repo>:latest
```

如果要固定版本：

```bash
docker pull ghcr.io/<owner>/<repo>:v1.0.0
docker run -d \
  --name yundo \
  --restart unless-stopped \
  -p 8080:8080 \
  -v yundo-cache:/tmp/cache \
  ghcr.io/<owner>/<repo>:v1.0.0
```

如果要覆盖默认缓存大小：

```bash
docker run -d \
  --name yundo \
  --restart unless-stopped \
  -p 8080:8080 \
  -v yundo-cache:/tmp/cache \
  ghcr.io/<owner>/<repo>:latest \
  --host 0.0.0.0 \
  --port 8080 \
  --cache-dir /tmp/cache \
  --cache-size 2147483648 \
  --frontend-dist ./frontend/dist
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

## 许可证

本项目使用 MIT License。见 [LICENSE](/D:/dev/yundo/LICENSE)。
