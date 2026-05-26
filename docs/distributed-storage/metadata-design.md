# 分布式存储元数据设计

本文定义 Yundo 自建分布式存储的元数据模型，包括文件、chunk、副本、上传会话、后台任务和数据库部署边界。

## 1. 数据库部署边界

第一阶段可以继续使用 SQLite，但 SQLite 模式只支持单控制平面实例：

- 单 API 实例。
- 单 scheduler。
- 单 repair worker。
- 单 GC worker。

多 API 实例、多 repair worker 或多 GC worker 部署时，必须迁移 PostgreSQL，并为任务认领、状态转换和副本修复使用事务、行级锁或等效并发控制。

推荐边界：

| 模式 | 支持能力 | 限制 |
| --- | --- | --- |
| SQLite | 单控制平面、单 worker、单机或少量 Storage Node | 不支持多 API 并发调度 |
| PostgreSQL | 多 API、多 worker、多 Storage Node | 需要任务认领和事务约束 |

## 2. 存储节点表

```sql
CREATE TABLE storage_nodes (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  endpoint TEXT NOT NULL,
  zone TEXT,
  status TEXT NOT NULL DEFAULT 'registered',
  capacity_bytes INTEGER NOT NULL,
  used_bytes INTEGER NOT NULL DEFAULT 0,
  active_uploads INTEGER NOT NULL DEFAULT 0,
  active_downloads INTEGER NOT NULL DEFAULT 0,
  active_replications INTEGER NOT NULL DEFAULT 0,
  avg_rtt_ms INTEGER,
  p95_rtt_ms INTEGER,
  packet_loss REAL,
  timeout_rate REAL,
  heartbeat_success_rate REAL,  -- 控制平面计算字段，非节点上报
  features TEXT,
  storage_version TEXT,
  public_download INTEGER NOT NULL DEFAULT 0,
  last_heartbeat_at DATETIME,
  created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

节点状态：

| 状态 | 含义 |
| --- | --- |
| registered | 已注册，但尚未通过心跳确认 |
| active | 正常读写 |
| degraded | 可读，谨慎写入或不接受新写入 |
| readonly | 只读，不接受新写入 |
| draining | 准备下线，后台迁移数据 |
| offline | 不参与调度 |

节点不应完全自行决定状态。节点上报磁盘、负载和网络指标，控制平面根据心跳 TTL、管理员状态和调度策略计算最终调度状态。

## 3. 文件表

```sql
CREATE TABLE files (
  id TEXT PRIMARY KEY,
  file_name TEXT NOT NULL,
  file_size INTEGER NOT NULL,
  content_type TEXT,
  chunk_size INTEGER NOT NULL,
  total_chunks INTEGER NOT NULL,
  sha256 TEXT,
  status TEXT NOT NULL DEFAULT 'uploading',
  replication_factor INTEGER NOT NULL DEFAULT 2,
  uploaded_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  expires_at DATETIME,
  updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

文件状态：

| 状态 | 含义 |
| --- | --- |
| uploading | 上传中 |
| partial_ready | 所有 chunk 至少有一个 ready 副本，允许下载，但副本数未达标 |
| ready | 所有 chunk 达到目标副本数 |
| repair_needed | 副本不足、校验失败或节点离线导致冗余不足 |
| deleting | 删除中 |
| deleted | 已删除 |
| failed | 上传失败或所有可用副本丢失 |

`partial_ready` 是可下载状态。对于临时文件箱，上传完成后只要每个 chunk 至少有一个 ready 副本，就可以返回成功并允许用户下载；后台 repair worker 继续补齐目标副本数。

## 4. 文件 chunk 表

```sql
CREATE TABLE file_chunks (
  id TEXT PRIMARY KEY,
  file_id TEXT NOT NULL,
  chunk_index INTEGER NOT NULL,
  size_bytes INTEGER NOT NULL,
  sha256 TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(file_id, chunk_index)
);
```

chunk 状态：

| 状态 | 含义 |
| --- | --- |
| pending | 尚未完成写入 |
| ready | 至少有一个可读副本 |
| repair_needed | 没有达到目标副本数或存在损坏副本 |
| deleting | 删除中 |
| deleted | 已删除 |

## 5. 分片副本表

```sql
CREATE TABLE chunk_replicas (
  chunk_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  object_key TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  sha256 TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'ready',
  verified_at DATETIME,
  created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (chunk_id, node_id)
);
```

副本状态：

| 状态 | 含义 |
| --- | --- |
| writing | 写入中，不可读 |
| ready | 可读 |
| corrupt | 校验失败 |
| missing | 节点缺失或对象不存在 |
| deleting | 删除中 |
| deleted | 已删除 |

## 6. 上传会话表

```sql
CREATE TABLE upload_sessions (
  id TEXT PRIMARY KEY,
  file_id TEXT NOT NULL,
  file_name TEXT NOT NULL,
  file_size INTEGER NOT NULL,
  content_type TEXT,
  chunk_size INTEGER NOT NULL,
  total_chunks INTEGER NOT NULL,
  replication_factor INTEGER NOT NULL DEFAULT 2,
  status TEXT NOT NULL DEFAULT 'active',
  created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  expires_at DATETIME NOT NULL
);
```

`upload_sessions` 记录的是上传请求的原始参数快照。complete 时从 session 拷贝到 `files` 表，而不是要求调用方再次提供，避免不一致。

上传会话状态：

| 状态 | 含义 |
| --- | --- |
| active | 可继续上传 |
| completing | 正在完成校验和状态收敛 |
| completed | 上传完成 |
| aborted | 用户取消或服务端取消 |
| expired | 会话过期 |
| failed | 无法恢复的上传失败 |

## 7. 上传 chunk 状态表

`upload_session_chunks` 记录上传过程中的每个 chunk 状态，避免把上传临时状态和正式文件 chunk 状态混在一起。

```sql
CREATE TABLE upload_session_chunks (
  upload_id TEXT NOT NULL,
  chunk_index INTEGER NOT NULL,
  size_bytes INTEGER,
  sha256 TEXT,
  status TEXT NOT NULL DEFAULT 'pending',
  node_id TEXT,
  object_key TEXT,
  error TEXT,
  created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (upload_id, chunk_index)
);
```

状态：

| 状态 | 含义 |
| --- | --- |
| pending | 尚未上传 |
| writing | 正在写入 Storage Node |
| uploaded | chunk 已写入并校验通过 |
| failed | chunk 上传失败，可重试 |
| conflict | 同一 chunk index 重传时 sha256 不一致 |
| deleting | abort 或过期清理中 |

幂等规则：

- 同一 `upload_id + chunk_index` 重传，sha256 相同则直接返回成功。
- sha256 不同必须拒绝，并标记为 `conflict`。
- complete 时必须确认所有 chunk 处于 `uploaded` 状态。

## 8. 删除补偿任务表

删除文件时不能只依赖同步删除。节点不可达或删除失败时，需要持久化 GC 任务，后续重试。

```sql
CREATE TABLE storage_gc_tasks (
  id TEXT PRIMARY KEY,
  file_id TEXT,
  chunk_id TEXT,
  node_id TEXT NOT NULL,
  object_key TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  retry_count INTEGER NOT NULL DEFAULT 0,
  max_retry INTEGER NOT NULL DEFAULT 10,
  next_retry_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  locked_by TEXT,
  locked_until DATETIME,
  last_error TEXT,
  created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

状态：

| 状态 | 含义 |
| --- | --- |
| pending | 等待删除 |
| running | 正在删除 |
| succeeded | 删除完成 |
| failed | 删除失败，等待下一次重试 |
| abandoned | 超过策略限制后人工处理 |

删除必须幂等：目标 object 不存在也应视为删除成功。

## 9. 副本修复任务表

副本复制、损坏修复和节点下线迁移统一为 repair task，由 repair worker 根据元数据持续收敛。

```sql
CREATE TABLE replica_repair_tasks (
  id TEXT PRIMARY KEY,
  file_id TEXT,
  chunk_id TEXT NOT NULL,
  source_node_id TEXT,
  target_node_id TEXT,
  reason TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  priority INTEGER NOT NULL DEFAULT 100,
  retry_count INTEGER NOT NULL DEFAULT 0,
  next_retry_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  locked_by TEXT,
  locked_until DATETIME,
  last_error TEXT,
  created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

`reason` 建议值：

| reason | 含义 |
| --- | --- |
| insufficient_replicas | 副本数不足 |
| corrupt_replica | 校验失败，需要从健康副本恢复 |
| missing_replica | 元数据存在但节点对象缺失 |
| node_draining | 节点下线迁移 |
| zone_rebalance | 副本分布不满足 zone 策略 |

多 worker 场景必须使用 `locked_by` 和 `locked_until` 实现任务认领，避免重复复制或重复迁移。

## 10. 校验字段语义

- `file_chunks.sha256` 必填，用于 chunk 级存储一致性、幂等上传和副本校验。
- `chunk_replicas.sha256` 必须与 `file_chunks.sha256` 一致。
- `files.sha256` 可选，用于整文件端到端校验。
- 如果服务端不重新按字节流读取所有 chunk，不能仅由 chunk hash 简单等价推导整文件 sha256。
- 第一阶段至少保证 chunk 级 sha256；后续可由前端提交整文件 sha256，或由服务端异步流式计算。

## 11. 逻辑外键关系

SQLite 阶段不强制物理外键，但以下逻辑外键关系必须在应用层保证，PostgreSQL 阶段可启用物理外键：

| 子表.field | 父表.field |
| --- | --- |
| `file_chunks.file_id` | `files.id` |
| `chunk_replicas.chunk_id` | `file_chunks.id` |
| `chunk_replicas.node_id` | `storage_nodes.id` |
| `upload_session_chunks.upload_id` | `upload_sessions.id` |
| `upload_session_chunks.node_id` | `storage_nodes.id` |
| `storage_gc_tasks.node_id` | `storage_nodes.id` |
| `replica_repair_tasks.chunk_id` | `file_chunks.id` |
| `replica_repair_tasks.source_node_id` | `storage_nodes.id` |
| `replica_repair_tasks.target_node_id` | `storage_nodes.id` |

## 12. 索引建议

PostgreSQL 模式建议增加：

```sql
CREATE INDEX idx_files_status_expires ON files(status, expires_at);
CREATE INDEX idx_file_chunks_file_id ON file_chunks(file_id, chunk_index);
CREATE INDEX idx_chunk_replicas_node ON chunk_replicas(node_id, status);
CREATE INDEX idx_chunk_replicas_verified ON chunk_replicas(verified_at) WHERE status = 'ready';
CREATE INDEX idx_upload_chunks_status ON upload_session_chunks(upload_id, status);
CREATE INDEX idx_gc_tasks_claim ON storage_gc_tasks(status, next_retry_at, locked_until);
CREATE INDEX idx_repair_tasks_claim ON replica_repair_tasks(status, priority, next_retry_at, locked_until);
```
