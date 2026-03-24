# Yundo

Yundo 是一个下载代理和下载历史看板项目。

- 服务端：Rust + Axum
- 前端：React + Vite
- 存储：SQLite
- 部署形态：Rust 服务端统一提供 API 和前端静态资源

## 功能

- `GET /api/proxy`：代理下载远程 `HTTP/HTTPS` 文件
- `GET /api/recent`：返回最近下载记录
- 本地磁盘缓存
- SQLite 下载历史与 7 天热度统计
- 基础 SSRF 拦截
- 前端静态资源托管

## 目录结构

```text
yundo/
├─ src/
│  └─ main.rs
├─ frontend/
│  ├─ src/
│  ├─ server/
│  ├─ index.html
│  ├─ package.json
│  └─ vite.config.ts
├─ .github/workflows/
│  └─ docker-image.yml
├─ .dockerignore
├─ Cargo.toml
├─ Dockerfile
├─ package.json
└─ README.md
```

## 环境要求

- Node.js 18+
- npm
- Rust stable
- Cargo

## 本地开发

安装依赖：

```bash
npm install
cargo build
```

前端开发模式：

```bash
npm run dev --workspace=frontend
```

完整本地运行：

```bash
npm run build
cargo run
```

默认地址：

- `http://127.0.0.1:8080`

可访问接口：

- `/`
- `/healthz`
- `/api/proxy`
- `/api/recent`

## Rust 服务参数

```bash
cargo run -- \
  --host 0.0.0.0 \
  --port 8080 \
  --cache-dir ./cache \
  --cache-size 1073741824 \
  --frontend-dist ./frontend/dist
```

参数说明：

- `--host`：监听地址，默认 `0.0.0.0`
- `--port`：监听端口，默认 `8080`
- `--cache-dir`：缓存目录，默认 `./cache`
- `--cache-size`：缓存大小上限，单位字节，必填
- `--frontend-dist`：前端构建产物目录，默认 `./frontend/dist`

## Docker 部署

项目已提供：

- [Dockerfile](/D:/dev/yundo/Dockerfile)
- [.dockerignore](/D:/dev/yundo/.dockerignore)
- [docker-image.yml](/D:/dev/yundo/.github/workflows/docker-image.yml)

镜像特点：

- 多阶段构建
- 前端单独构建
- Rust `release` 构建
- 运行层使用 `distroless`
- SQLite 使用 bundled 静态构建

本地构建镜像：

```bash
docker build -t yundo:latest .
```

本地运行：

```bash
docker run --rm -p 8080:8080 yundo:latest
```

注意：

- 现在必须显式指定缓存容量
- Dockerfile 内置默认值为 `1073741824`，即 `1 GiB`

如果需要持久化缓存和数据库：

```bash
docker run --rm -p 8080:8080 -v yundo-cache:/tmp/cache yundo:latest
```

容器内缓存目录为：

```text
/tmp/cache
```

## 通过 GitHub Docker 镜像部署

仓库已配置 GitHub Actions 自动构建并推送镜像到 GitHub Container Registry。

镜像地址格式：

```text
ghcr.io/<owner>/<repo>:<tag>
```

例如仓库是 `foo/yundo`，则镜像地址通常为：

```text
ghcr.io/foo/yundo:latest
```

### 1. 触发镜像构建

以下情况会触发 workflow：

- push 到 `master`
- push tag，例如 `v1.0.0`
- pull request
- 手动触发 `workflow_dispatch`

推送规则：

- `pull_request` 只构建，不推送
- `master` 会推送分支镜像
- 默认分支会带 `latest`
- tag 会推送同名 tag 镜像

### 2. 在 GitHub Packages 中确认镜像

构建完成后，可在仓库对应的 `Packages` 页面查看镜像。

也可以使用命令拉取：

```bash
docker pull ghcr.io/<owner>/<repo>:latest
```

### 3. 登录 GHCR

如果镜像是私有的，需要先登录：

```bash
echo <GITHUB_TOKEN> | docker login ghcr.io -u <GITHUB_USERNAME> --password-stdin
```

要求：

- `GITHUB_TOKEN` 或 Personal Access Token 需要有 `read:packages`

### 4. 服务器上拉取并运行

拉取镜像：

```bash
docker pull ghcr.io/<owner>/<repo>:latest
```

启动容器：

```bash
docker run -d \
  --name yundo \
  --restart unless-stopped \
  -p 8080:8080 \
  -v yundo-cache:/tmp/cache \
  ghcr.io/<owner>/<repo>:latest
```

部署完成后访问：

```text
http://<server-ip>:8080
```

### 5. 更新部署

当 GitHub Actions 构建出新镜像后，在服务器执行：

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

### 6. 使用指定版本部署

如果希望固定版本而不是追 `latest`，建议使用 tag：

```bash
docker pull ghcr.io/<owner>/<repo>:v1.0.0
docker run -d \
  --name yundo \
  --restart unless-stopped \
  -p 8080:8080 \
  -v yundo-cache:/tmp/cache \
  ghcr.io/<owner>/<repo>:v1.0.0
```

如果你不想使用镜像内置的 `1 GiB`，可以覆盖启动命令：

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

查询参数：

- `url`：目标下载地址，必填

示例：

```text
/api/proxy?url=https%3A%2F%2Fexample.com%2Ffile.zip
```

行为：

- 仅允许 `http` 和 `https`
- 支持透传 `Range`
- 下载文件名会尽量保持与源链接一致

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

## 缓存和数据库

运行后缓存目录下会生成：

- `*.data`
- `*.meta`
- `proxy.db`

数据表：

- `download_history`
- `download_events`

## 已验证

- `cargo build`
- `npm run build`
- `GET /healthz`
- `GET /api/recent`

## 已知事项

- 前端仍有部分历史文案乱码，属于展示问题，不影响服务运行
- SSRF 防护目前是基础规则，若公网部署，建议继续加强域名解析和私网 IP 校验
- 当前环境未安装 Docker CLI，因此未在本机实际执行 `docker build`
