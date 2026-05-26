# Yundo 自建分布式存储资源技术方案

本文是 Yundo FileBox 从单机本地磁盘演进为自建分布式 chunk 存储的总览文档。详细设计按主题拆分到 `docs/distributed-storage/` 目录，便于独立评审和后续实现。

## 1. 背景与目标

Yundo 当前临时文件箱基于单机本地磁盘实现，文件直接写入 `cache/filebox`，元数据记录在 SQLite 的 `filebox_files` 表中。该模式部署简单，但在容量扩展、节点故障恢复、多地域接入和弱网传输方面存在限制。

本方案优先采用自建存储节点，而不是直接依赖 S3/MinIO 等对象存储。目标是在现有 FileBox 能力基础上，演进为可横向扩展、可断点恢复、可多副本容灾，并能适应高延迟、高丢包率网络环境的分布式存储资源系统。

核心目标：

- 多个自建存储节点共同承载文件数据。
- 文件采用 chunk 存储，上传、下载、复制均支持断点恢复。
- 支持节点健康检查、容量调度、副本调度和故障切换。
- 高延迟、高丢包网络下，失败只重传失败 chunk，避免整文件重传。
- 保留现有临时文件箱使用体验，逐步迁移现有本地存储实现。

非目标：

- 第一阶段不实现强一致分布式文件系统。
- 第一阶段不自研复杂二进制传输协议。
- 第一阶段不实现纠删码，优先使用多副本保证可用性。
- 第一阶段不支持多控制平面实例；多 API/多 worker 必须迁移 PostgreSQL 并引入任务认领机制。

## 2. 当前系统现状

当前相关模块：

- `src/filebox.rs`：上传、分片上传、远程转存、下载、删除、过期清理。
- `src/cache.rs`：本地缓存和文件箱空间统计、缓存淘汰。
- `src/state.rs`：应用状态、缓存目录初始化、SQLite 表初始化。
- `frontend/src/pages/FileBox.tsx`：文件箱前端上传、下载、删除交互。

当前写入路径：

```text
HTTP upload
  -> filebox handler
  -> cache/filebox/{id}
  -> filebox_files metadata
```

当前下载路径：

```text
GET /api/filebox/download/:id
  -> query filebox_files
  -> open cache/filebox/{id}
  -> stream response
```

主要限制：

- 文件和元数据绑定在单个服务实例和单个磁盘目录。
- 多实例部署时无法共享文件数据。
- 节点故障会导致本地文件不可用。
- 当前前端分片上传只解决浏览器到 API 的请求体大小问题，后端仍合并为单个本地文件。
- 弱网环境下缺少 chunk 校验、缺少失败 chunk 恢复状态、缺少多节点下载切换。

## 3. 总体架构

目标架构分为控制平面和数据平面。

```text
Frontend / Client
  |
Yundo API / Control Plane
  |-- Metadata DB
  |-- Upload Session Service
  |-- Storage Scheduler
  |-- Node Health Manager
  |-- Replica Repair Worker
  |-- GC Worker
  |
Storage Nodes / Data Plane
  |-- Local chunk object store
  |-- Chunk upload/download API
  |-- Checksum validation
  |-- Node-to-node replication
```

控制平面职责：

- 创建上传会话。
- 决定文件 chunk 大小和副本数。
- 选择写入节点和备用节点。
- 维护文件、chunk、副本、节点、任务状态元数据。
- 处理下载调度和节点故障切换。
- 后台执行副本补齐、修复、删除补偿和节点下线迁移。

数据平面职责：

- 接收 chunk 写入并落盘。
- 对 chunk 计算和校验 sha256。
- 按安全 object key 读取 chunk。
- 删除 chunk。
- 支持节点间复制。
- 上报磁盘、负载、网络质量和健康状态。

## 4. 设计原则

- **先 chunk 化，再多节点化**：先消除后端合并单文件的存储模型，再引入独立 Storage Node 和多副本。
- **先抽象 StorageBackend，再拆独立进程**：先在代码中建立本地存储 backend 抽象，降低拆分 Storage Node 的一次性风险。
- **chunk 级幂等与恢复**：上传、下载、复制、删除均以 chunk 为最小恢复单位。
- **元数据驱动收敛**：副本补齐、损坏修复、节点下线迁移和删除补偿都由持久化任务驱动。
- **弱网优先稳定性**：动态并发、指数退避和副本切换优先保证完成率，避免抖动和重试风暴。
- **安全默认拒绝**：节点注册、内部接口和 object key 均按白名单、短时签名、token 绑定和路径规范化设计。

## 5. 专题文档索引

- [元数据设计](distributed-storage/metadata-design.md)
  - 文件、chunk、副本、上传会话、上传 chunk 状态、GC task、repair task。
  - SQLite 与 PostgreSQL 能力边界。
  - 文件状态、chunk 状态、副本状态和任务状态语义。

- [API 设计](distributed-storage/api-design.md)
  - 上传会话、chunk 上传、上传状态、complete、abort。
  - 文件下载、下载计划。
  - Storage Node 内部 chunk API。
  - object key 传递与路径安全约束。

- [节点发现与安全](distributed-storage/node-discovery-and-security.md)
  - 节点注册、心跳、服务发现视图、状态收敛、本地缓存。
  - 内部认证、mTLS、token 绑定、endpoint SSRF 防护、审计。

- [传输与调度](distributed-storage/transfer-scheduling.md)
  - chunk 大小策略、sha256 校验、弱网恢复、动态并发、超时重试。
  - 上传节点选择、下载副本选择、副本放置策略。

- [运维与修复](distributed-storage/operations-and-repair.md)
  - 统一 repair worker、副本补齐、corrupt/missing 修复、draining 迁移。
  - 删除补偿、过期清理、任务认领和多 worker 并发控制。

- [迁移、实施与测试](distributed-storage/migration-implementation-testing.md)
  - 对现有代码的改造建议。
  - StorageBackend 抽象。
  - 兼容迁移、实施阶段、测试计划、风险与应对。

- [前端改造](distributed-storage/frontend-migration.md)
  - 前端 API 对接变更。
  - chunk sha256 计算。
  - 断点恢复 UI。
  - chunk size 和 upload_id 动态获取。

## 6. 推荐实施路线

### 阶段一：单节点 chunk 化 + 上传会话持久化

目标：不引入多节点，先把后端从单文件合并改为 chunk 存储。

关键工作：

- 增加 `files`、`file_chunks`、`chunk_replicas`、`upload_sessions`、`upload_session_chunks` 表。
- 实现上传会话、chunk sha256 校验、幂等 chunk 上传、状态查询和 complete 校验。
- 下载时按 chunk 顺序流式输出，不再依赖合并后的单个本地文件。
- 保持现有前端使用体验。

### 阶段二：引入 StorageBackend 抽象

目标：先在代码层面解耦 FileBox handler 与本地磁盘实现。

建议分层：

```text
filebox handlers
  -> upload session service
  -> metadata repository
  -> storage scheduler
  -> StorageBackend
```

第一版 `StorageBackend` 使用本地磁盘实现，为后续远程 Storage Node 做准备。

### 阶段三：拆出独立 Storage Node

目标：API 不再直接写文件，由 Storage Node 落盘。

关键工作：

- 增加节点内部 chunk API。
- 增加节点注册、注册重试和心跳。
- 实现控制平面的服务发现缓存。
- API 通过 storage client 写入节点。
- 支持 `all` 模式兼容单机部署。

### 阶段四：多节点副本与自动修复

目标：支持多个自建节点和多副本。

关键工作：

- 上传节点评分和副本放置。
- repair worker 根据目标副本数持续补齐副本。
- 下载失败自动切换副本。
- 删除补偿和节点下线迁移。

### 阶段五：弱网优化

目标：提升高延迟、高丢包环境下的成功率和吞吐。

关键工作：

- 客户端动态并发窗口。
- chunk 级指数退避重试。
- 上传状态恢复。
- 下载副本超时切换。
- RTT、丢包率、失败率采集。

### 阶段六：协议与运维增强

目标：提升跨地域弱网和生产可运维能力。

关键工作：

- 评估 HTTP/3/QUIC。
- 增加节点下线的运维自动化（自动 draining 进度跟踪、完成确认、跨地域节点迁移）。
- 增加管理 API、前端节点状态页。
- 增加审计日志和告警指标。

## 7. 推荐结论

推荐先实现“单节点 chunk 化 + 上传会话持久化”，再引入 `StorageBackend` 抽象，然后拆出“自建 Storage Node”，随后引入“多节点副本和 repair worker”，再进行“弱网优化”，最后实施“协议与运维增强”。

优先级最高的技术决策：

- 文件必须 chunk 化。
- chunk 必须有 sha256。
- 上传会话和每个 chunk 的状态必须可恢复。
- `partial_ready` 文件允许下载，但表示副本数未达标。
- 下载必须能按 chunk 切换副本。
- 节点状态、副本状态和后台任务状态必须进入元数据。
- SQLite 模式只支持单控制平面，多控制平面必须迁移 PostgreSQL。
- 内部 object key 不能直接作为未校验路径使用。

完成这些基础能力后，再引入 HTTP/3/QUIC、跨地域调度、节点下线迁移和更复杂的数据修复机制。
