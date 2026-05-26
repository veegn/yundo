# 运维与修复设计

本文定义后台任务、repair worker、删除补偿、过期清理、节点 draining 和多 worker 并发控制。

## 1. 后台任务原则

分布式存储中的副本补齐、损坏修复、删除补偿和节点下线迁移都必须持久化，不能只依赖内存任务。

原则：

- 任务必须幂等。
- 任务失败后可重试。
- 节点不可达时不丢失任务。
- 多 worker 场景必须支持任务认领。
- 用户请求路径不应长时间等待后台收敛。

## 2. 统一 repair worker 模型

上传完成后不单独依赖一次性复制任务，而是由 repair worker 根据目标副本数持续收敛。

repair worker 处理：

- 副本数不足。
- 副本 sha256 校验失败。
- 元数据存在但节点对象缺失。
- 节点离线导致冗余不足。
- 节点 draining 迁移。
- zone 分布不满足策略。

统一流程：

```text
scan metadata or consume repair task
  -> identify chunk needing repair
  -> select healthy source replica
  -> select target node
  -> copy chunk
  -> verify sha256
  -> insert or update chunk_replicas
  -> update chunk/file status
  -> mark repair task succeeded
```

## 3. 副本补齐

上传 complete 后：

```text
if every chunk has >= 1 ready replica:
  file.status = partial_ready
  enqueue repair tasks for chunks below replication_factor
else:
  file.status = failed or uploading
```

repair worker 补齐后：

```text
if every chunk ready replicas >= replication_factor:
  file.status = ready
else if every chunk ready replicas >= 1:
  file.status = partial_ready or repair_needed
else:
  file.status = failed
```

`partial_ready` 文件允许下载。`ready` 只表示达到目标副本数。

## 4. 损坏副本修复

触发来源：

- 下载时 sha256 校验失败。
- 周期性抽样校验失败。
- Storage Node verify API 返回 invalid。
- 节点恢复后的轻量校验发现异常。

流程：

```text
mark replica = corrupt
  -> find another ready replica
  -> enqueue repair task reason=corrupt_replica
  -> copy to target node or overwrite corrupt node object
  -> verify sha256
  -> mark new replica ready
  -> enqueue GC task for corrupt object if needed
```

如果没有健康源副本：

```text
mark chunk = failed
mark file = failed or repair_needed
alert operator
```

## 5. missing 副本修复

触发来源：

- 节点上报对象不存在。
- verify API 返回 `exists=false`。
- 节点磁盘恢复后对比元数据发现缺失。

流程：

```text
mark replica = missing
  -> if enough other ready replicas:
       enqueue repair task
     else:
       mark file repair_needed or failed
```

## 6. 删除补偿

删除文件流程：

```text
mark file = deleting
  -> enumerate chunk_replicas
  -> create storage_gc_tasks for each replica
  -> worker deletes objects idempotently
  -> mark replicas deleted
  -> mark chunks deleted
  -> mark file deleted
```

删除规则：

- Storage Node 不可达时，GC task 保留为 `failed` 并设置 `next_retry_at`。
- object 不存在时，视为删除成功。
- 文件元数据不应在所有删除任务完成前物理删除；可以先标记 `deleted` 并保留短期 tombstone。
- 过期文件删除和用户主动删除复用同一 GC task 机制。

## 7. 过期清理

周期扫描：

```text
find files where expires_at < now and status not in (deleting, deleted)
  -> mark deleting
  -> enqueue storage_gc_tasks
```

清理还应处理：

- 过期的 upload session。
- aborted upload session 的临时 chunks。
- 已 succeeded 的 GC task 归档或清理。
- 长期 failed 的 GC task 告警。

### 7.1 上传会话过期处理

```text
find upload_sessions where expires_at < now and status = 'active'
  -> mark expired
  -> enumerate uploaded chunks for each expired session
  -> enqueue storage_gc_tasks for temporary chunks
```

过期检测策略：

- 采用定时扫描（复用已有过期清理周期）加 lazy check（访问时检查）组合。
- 客户端在会话过期后尝试上传 chunk 时，返回 410 Gone。
- 过期清理与 abort 清理复用同一 GC task 机制。

## 8. 节点 draining

当节点进入 `draining`：

```text
stop new writes to node
  -> find ready replicas on node
  -> for each replica:
       if removing it would violate replication_factor:
         enqueue repair task reason=node_draining
       else:
         enqueue GC task or keep until final cutover
  -> wait repair tasks completed
  -> delete old replicas from draining node
  -> mark node offline or removable
```

注意：

- draining 期间节点仍可作为下载源，除非磁盘或网络异常。
- draining 不应直接删除唯一副本。
- draining 任务可暂停和恢复。

## 9. 节点恢复

节点从 offline 恢复后先进入 `degraded`：

```text
heartbeat restored
  -> mark degraded
  -> check storage directory read/write
  -> sample verify chunks
  -> compare used space with metadata estimate
  -> observe N stable heartbeat windows
  -> mark active
```

如果发现大量 missing 或 corrupt：

- 标记相关 replicas。
- 创建 repair tasks。
- 该节点暂不接受新写入。

## 10. 任务认领与并发控制

多 worker 场景必须避免重复执行同一任务。

任务认领字段：

```text
status
locked_by
locked_until
retry_count
next_retry_at
```

认领流程：

```text
worker selects pending/failed tasks where next_retry_at <= now and lock expired
  -> atomically set status=running, locked_by=worker_id, locked_until=now+lease
  -> execute task
  -> on success mark succeeded
  -> on failure retry_count += 1, next_retry_at = backoff, status=failed
```

PostgreSQL 可使用：

```sql
SELECT ... FOR UPDATE SKIP LOCKED
```

SQLite 模式不建议运行多个 worker。

## 11. 重试与退避

建议策略：

| retry_count | next_retry_at |
| --- | --- |
| 0 | now + 1s |
| 1 | now + 5s |
| 2 | now + 30s |
| 3 | now + 2m |
| 4 | now + 10m |
| >=5 | now + 1h，必要时告警 |

所有重试应带 jitter，避免集群恢复时同时打爆节点。

## 12. Reconcile 周期任务

除显式 repair task 外，还需要周期性 reconcile：

- `ready` 文件是否达到目标副本数。
- `partial_ready` 文件是否长期未补齐副本。
- 是否存在 `corrupt` 或 `missing` 副本。
- 是否有节点离线导致副本数不足。
- 是否有 dangling metadata 或 orphan object。
- 是否有长期 failed GC task。

reconcile 只负责发现差异并创建任务，不应在扫描事务内执行大量复制或删除。

## 13. repair task 去重策略

避免同一 chunk 被多次创建 repair task：

- 创建 repair task 前，检查是否已存在同一 `chunk_id + reason` 且状态为 pending/running 的任务。
- PostgreSQL 可使用条件唯一索引精确防重：

```sql
CREATE UNIQUE INDEX idx_repair_dedup
  ON replica_repair_tasks(chunk_id, reason)
  WHERE status IN ('pending', 'running');
```

- SQLite 不支持条件唯一索引，应在应用层做幂等检查。
- reconcile 扫描发现差异时，如果已有未完成的任务，跳过创建。

## 14. 容量配额管理

多节点架构下的容量计算规则：

- 总可用容量为所有 active 节点的 `capacity_bytes` 之和。
- 用户可见配额可由控制平面配置逻辑上限，不必等于物理总量。
- 上传会话 init 时应检查“剩余空间是否足够存放整个文件”，但只作为软限制（其他并发上传可能同时占用空间）。
- 多副本场景下，1GB 文件 x 2 副本 = 实际占用 2GB。用户可见配额按原始文件大小计算，存储调度按实际副本占用计算。

## 15. 告警建议

建议告警：

- 文件进入 `failed`。
- `repair_needed` 文件数量持续增长。
- repair task 长期失败。
- GC task 长期失败。
- 节点 heartbeat 超时。
- 节点恢复后 corrupt/missing 比例异常。
- 存储水位超过阈值。

## 16. 运维操作

建议提供管理 API 或管理界面：

- 查看节点列表和状态。
- 设置节点 readonly、draining、offline。
- 查看 repair task 和 GC task。
- 手动触发文件或节点 reconcile。
- 手动重试 failed task。
- 查看审计日志。
