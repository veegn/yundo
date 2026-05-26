# 迁移、实施与测试计划

本文定义对现有 FileBox 的改造建议、兼容迁移策略、实施阶段、测试计划和主要风险。

## 1. 当前代码改造范围

当前相关模块：

- `src/filebox.rs`：上传、分片上传、远程转存、下载、删除、过期清理。
- `src/cache.rs`：本地缓存和文件箱空间统计、缓存淘汰。
- `src/state.rs`：应用状态、缓存目录初始化、SQLite 表初始化。
- `frontend/src/pages/FileBox.tsx`：文件箱前端上传、下载、删除交互。

当前 `src/filebox.rs` 中以下逻辑应逐步迁出：

- 直接 `File::create` 写入本地文件。
- 直接 `File::open` 下载本地文件。
- 直接 `fs::remove_file` 删除文件。
- `upload-complete` 阶段合并本地 chunk 为单个文件。

## 2. 推荐分层

先抽象 `StorageBackend`，再拆独立 Storage Node。

建议分层：

```text
filebox handlers
  -> upload session service
  -> metadata repository
  -> storage scheduler
  -> StorageBackend
```

第一阶段 `StorageBackend` 使用本地磁盘实现。后续独立 Storage Node 可以作为远程 backend，不需要重写 FileBox handler 的核心逻辑。

建议新增模块：

```text
src/storage/mod.rs
src/storage/backend.rs
src/storage/local.rs
src/storage/node.rs
src/storage/client.rs
src/storage/scheduler.rs
src/storage/repair.rs
src/storage/gc.rs
src/storage/routes.rs
src/uploads.rs
```

## 3. StorageBackend 抽象

建议能力：

```text
put_chunk(object_key, bytes, expected_sha256) -> ChunkWriteResult
get_chunk(object_key) -> stream
delete_chunk(object_key) -> idempotent result
verify_chunk(object_key, expected_sha256) -> VerifyResult
```

实现：

- `LocalStorageBackend`：本地磁盘 chunk object store。
- `RemoteStorageNodeBackend`：通过内部 HTTP API 访问 Storage Node。

路径安全：

- backend 接收的 object key 必须是已校验的逻辑 key。
- 本地 backend 根据解析后的 file_id、upload_id、chunk_index 生成最终路径。
- 不允许直接把 object key 字符串拼接为文件路径。

## 4. 兼容迁移策略

第一阶段应支持现有本地文件继续下载。

迁移步骤：

1. 新增新表，不删除 `filebox_files`。
2. 新文件走新 chunk 存储。
3. 旧文件保留旧下载路径。
4. 后台迁移任务逐步把旧文件拆分为 chunk。
5. 迁移完成后，为旧文件写入 `files`、`file_chunks`、`chunk_replicas`。
6. 验证通过后删除旧本地单文件。

兼容下载逻辑：

```text
if file exists in new files table:
  use distributed chunk download
else:
  fallback to legacy cache/filebox/{id}
```

兼容删除逻辑：

```text
if file exists in new files table:
  mark deleting and enqueue GC tasks
else:
  delete legacy filebox_files row and cache/filebox/{id}
```

兼容列表逻辑：

```text
list API:
  query new files table (status not in deleting, deleted)
  union query legacy filebox_files (expires_at >= now)
  merge by uploaded_at desc
```

迁移期间 list API 必须同时查询新旧两张表，确保用户可以看到所有文件。

## 5. 配置项

新增建议配置：

```text
--node-mode api|storage|all
--node-id node-a
--node-endpoint https://node-a.internal.example
--storage-dir ./cache/storage
--discovery-mode builtin
--api-endpoint https://api.example.com
--node-zone cn-east-1a
--node-heartbeat-interval 30s
--node-register-retry-interval 10s
--default-chunk-size 16MiB
--min-chunk-size 4MiB
--max-chunk-size 64MiB
--default-replication-factor 2
--upload-session-ttl 24h
--node-heartbeat-ttl 90s
--internal-token <token>
--max-upload-concurrency 6
--weak-network-loss-threshold 0.10
```

部署模式：

| 模式 | 用途 |
| --- | --- |
| api | 只运行控制平面 |
| storage | 只运行存储节点 |
| all | 单机兼容模式，同时运行 API 和存储节点 |

`all` 模式下，控制平面可以绕过网络调用直接使用 `LocalStorageBackend`，无需走注册/心跳/内部 HTTP 链路。这避免了 loopback 地址与安全策略的冲突，也是 `StorageBackend` 抽象的核心价值之一。只有当 `--node-mode` 为 `api` 或 `storage` 时，才需要启动节点注册和心跳。

## 6. 实施阶段

### 阶段一：单节点 chunk 化 + 上传会话持久化

目标：不引入多节点，先把单文件存储改为 chunk 存储。

工作项：

- 增加 `files`、`file_chunks`、`chunk_replicas`、`upload_sessions`、`upload_session_chunks` 表。
- 实现上传会话。
- 实现 chunk 上传、sha256 校验、状态查询和 complete。
- complete 不再合并为单个本地文件。
- 下载时按 chunk 顺序流式输出。
- 保持前端现有体验。

验收：

- 大文件上传失败后可只重传缺失 chunk。
- 服务端重启后可继续查询上传状态。
- 下载内容与原文件一致。
- `partial_ready` 文件可下载。

### 阶段二：StorageBackend 抽象

目标：解耦 FileBox handler 与本地磁盘。

工作项：

- 定义 `StorageBackend` trait 或等效接口。
- 实现 `LocalStorageBackend`。
- FileBox 上传、下载、删除通过 backend 操作 chunk。
- object key 由服务端生成并规范化。

验收：

- 业务 handler 不再直接拼接 chunk 文件路径。
- 本地 backend 能通过相同接口完成上传、下载、删除和 verify。

### 阶段三：存储节点进程

目标：拆出独立 Storage Node。

工作项：

- 增加节点内部 chunk API。
- 增加节点注册接口和注册重试。
- 增加节点心跳。
- 实现控制平面的服务发现缓存。
- API 通过 storage client 写入节点。
- 支持 `all` 模式兼容单机部署。

验收：

- API 节点不直接写文件，文件由 Storage Node 落盘。
- Storage Node 重启后可自动重新注册并恢复心跳。
- 节点离线后不再被选择为上传目标。

### 阶段四：多节点调度与副本

目标：支持多个自建节点和多副本。

工作项：

- 实现上传节点评分。
- 调度器基于服务发现视图过滤节点。
- 实现 repair worker 持续补齐副本。
- 实现下载失败自动切换副本。
- 实现 storage GC task。

验收：

- 单个节点下线后，已有双副本文件仍可下载。
- 节点进入 `draining` 后不再接收新写入。
- 副本不足时系统能自动补齐。
- 删除失败后节点恢复可以补删。

### 阶段五：弱网优化

目标：优化高延迟、高丢包环境下的成功率和吞吐。

工作项：

- 客户端动态并发窗口。
- chunk 级指数退避重试。
- 上传状态恢复。
- 下载副本超时切换。
- 采集 RTT、丢包率和失败率。

验收：

- 模拟 200ms RTT、10% 丢包时，大文件上传可完成。
- 失败中断后，恢复上传不会重传已完成 chunk。
- 下载时单个副本失败可切换到其他副本。

### 阶段六：协议与运维增强

目标：提升跨地域弱网和生产运维能力。

工作项：

- 评估 HTTP/3/QUIC。
- 增加节点下线迁移。
- 增加管理 API 和前端节点状态页。
- 增加审计日志和告警指标。

## 7. 单元测试计划

- chunk index 计算。
- chunk size 边界。
- sha256 校验。
- object key 规范化。
- 节点评分。
- 上传状态合并。
- 副本选择。
- repair task 认领。
- GC task 幂等删除。

## 8. 集成测试计划

- 单节点上传下载。
- 上传中断后恢复。
- chunk sha256 不一致时拒绝。
- 同一 chunk 重传 sha256 相同时幂等成功。
- 同一 chunk 重传 sha256 不同时返回冲突。
- 多节点副本复制。
- 下载节点失败后切换副本。
- 过期文件删除所有副本。
- 删除时节点不可达，节点恢复后补删。
- 节点 draining 后副本迁移完成。

## 9. 弱网测试计划

使用 `tc netem` 或等效网络模拟工具。

测试场景：

- 高 RTT：100ms、200ms、500ms。
- 丢包：5%、10%、20%。
- 抖动：50ms、100ms。
- 带宽限制：1Mbps、5Mbps、20Mbps。
- 单 Storage Node 超时。
- 下载过程中副本节点断开。

关键指标：

- 上传成功率。
- 平均恢复耗时。
- 重传数据量。
- 下载首字节时间。
- 副本修复完成时间。
- repair task 积压数量。
- GC task 积压数量。

## 10. 风险与应对

| 风险 | 影响 | 应对 |
| --- | --- | --- |
| 元数据和实际 chunk 不一致 | 文件不可读或空间泄漏 | 写入先标记状态，校验成功后提交 ready；周期性 reconcile |
| 控制平面单点故障 | 上传和调度不可用 | SQLite 阶段接受单点；多实例部署迁移 PostgreSQL |
| 多 worker 重复执行任务 | 重复复制或重复删除 | 使用任务认领、锁租约和幂等操作 |
| 弱网下重试放大流量 | 拥塞加剧 | 指数退避、动态并发、失败率限流 |
| 节点磁盘损坏 | 副本丢失 | 定期校验、副本修复、跨 zone 放置 |
| 删除失败导致空间泄漏 | 存储成本增加 | 删除任务持久化，节点恢复后补删 |
| endpoint 注册被滥用 | SSRF 或内部服务探测 | endpoint 白名单、禁止敏感网段、不跟随重定向 |
| object key 路径穿越 | 任意文件读写 | object key 结构化校验，服务端生成落盘路径 |
| 实现复杂度过高 | 难以稳定交付 | 分阶段交付，先完成单节点 chunk 化和 StorageBackend 抽象 |

## 11. 推荐结论

推荐先实现“单节点 chunk 化 + 上传会话持久化”，再引入 `StorageBackend` 抽象，随后拆出 Storage Node，最后引入多节点副本、repair worker 和弱网调度。

这条路径能保持当前 FileBox 功能连续可用，同时逐步建立真正的分布式存储能力。
