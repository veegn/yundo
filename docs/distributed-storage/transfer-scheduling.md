# 传输与调度设计

本文定义 chunk 大小策略、弱网传输、动态并发、重试、下载副本切换和节点调度评分。

## 1. 文件与 chunk 模型

文件不再以后端单文件形式存储，而是拆成固定大小 chunk。

```text
file_id
  chunk_000000
  chunk_000001
  chunk_000002
```

chunk size 在上传会话创建时确定，并写入元数据。后续同一文件内保持固定大小，最后一个 chunk 可小于标准 chunk size。

## 2. Chunk 大小策略

推荐策略：

| 场景 | Chunk 大小 |
| --- | --- |
| 高丢包或移动网络 | 4 MiB - 8 MiB |
| 默认普通公网 | 16 MiB |
| 大文件、稳定高速链路 | 32 MiB - 64 MiB |

约束：

- chunk size 必须小于反向代理请求体限制。
- 如果经过 Cloudflare 100MB 上传限制，应保留足够余量，不建议超过 64 MiB。
- chunk 越小，弱网重传成本越低，但 DB 记录、HTTP 请求和调度开销越高。
- chunk 越大，吞吐更好，但失败重传成本更高。
- 不建议第一阶段固定 72 MiB；可作为稳定链路上限配置，而不是默认值。

建议默认值：

```text
default_chunk_size = 16MiB
min_chunk_size = 4MiB
max_chunk_size = 64MiB
```

## 3. Checksum 策略

chunk sha256 是必需能力：

- 客户端上传 chunk 时提交 `X-Chunk-Sha256`。
- StorageBackend 或 Storage Node 写入后计算 sha256。
- sha256 匹配后才能把 chunk 标记为 `uploaded` 或副本标记为 `ready`。
- 下载时如果发现副本 sha256 不匹配，标记该副本为 `corrupt` 并切换其他副本。

整文件 sha256 是可选能力：

- 前端可以在上传前或上传过程中计算整文件 sha256。
- 服务端也可以异步按 chunk 顺序流式计算整文件 sha256。
- 不能简单用 chunk hash 拼接后计算来替代原始文件 sha256，除非协议明确使用树哈希。

## 4. 分片级恢复

高延迟、高丢包网络下，任何失败都应限制在 chunk 范围内。上传、下载、节点复制均以 chunk 为最小重试单位。

要求：

- 客户端可查询已完成 chunk。
- 服务端可幂等接收同一 chunk。
- chunk 写入完成前状态为 `writing`。
- 校验通过后状态变为 `uploaded` 或 `ready`。
- 同一 chunk 重传时，sha256 相同可直接返回成功，sha256 不同应拒绝并记录冲突。

## 5. 动态并发窗口

上传初始并发建议为 2。运行中根据成功率、RTT 和超时动态调整。

调整规则：

```text
连续成功 8 个 chunk 且平均 RTT 稳定 -> 并发 +1，最高 6
出现 2 次连续超时或错误率 > 20% -> 并发减半，最低 1
packet_loss > 10% -> 最高并发限制为 2
packet_loss > 20% -> 强制并发 1
```

并发窗口应是客户端和服务端协同结果：

- 服务端 init 返回 `concurrency_hint`。
- 客户端根据实时上传结果调整。
- 服务端可在状态接口返回新的并发建议。
- Storage Node 过载时可返回 429/503 触发客户端降速。

## 6. 超时与重试

建议参数：

| 参数 | 建议值 |
| --- | --- |
| 连接超时 | 10s |
| 单 chunk 写入超时 | 60s - 180s |
| 单 chunk 最大重试 | 5 次 |
| 退避策略 | 指数退避 + jitter |

退避示例：

```text
1s, 2s, 4s, 8s, 16s + 0-500ms jitter
```

重试规则：

- sha256 不匹配可重试同一 chunk。
- sha256 冲突不可自动重试，需要客户端重新读取原始文件 chunk 后再上传。
- 429/503 应降低并发并退避。
- 404 上传会话不存在或过期时，应重新 init。

## 7. 多副本下载切换

下载时为每个 chunk 选择一个优先副本。如果读取失败、超时或校验失败，立即切换到其他副本。

选择优先级：

```text
ready 副本
  -> active 节点
  -> RTT 较低
  -> packet_loss 较低
  -> active_downloads 较少
```

切换规则：

- 当前副本连接失败：尝试下一个副本。
- 当前副本超时：标记该节点失败计数，尝试下一个副本。
- 当前副本 sha256 不匹配：标记 replica = corrupt，尝试下一个副本，并创建 repair task。
- 所有副本失败：返回下载失败，并将文件或 chunk 标记为 `repair_needed` 或 `failed`。

## 8. 上传节点选择

过滤条件：

- 节点状态必须为 `active`。
- 剩余空间必须大于计划写入数据和预留水位。
- 节点磁盘状态必须正常。
- 节点最近心跳不能过期。
- 节点必须具备 `chunk-write` 和 `checksum` 特性。

评分公式：

```text
score =
  100 * free_ratio
- 0.05 * avg_rtt_ms
- 0.02 * p95_rtt_ms
- 200 * packet_loss
- 100 * timeout_rate
- 2 * active_uploads
- 1 * active_downloads
- 2 * active_replications
```

其中 `free_ratio = (capacity_bytes - used_bytes) / capacity_bytes`。

当需要多个副本时，应尽量选择不同 `zone` 的节点，避免同机房或同链路故障导致副本同时不可用。

## 9. 下载副本选择

下载选择偏向低延迟和低丢包：

```text
score =
  100
- 0.1 * avg_rtt_ms
- 0.03 * p95_rtt_ms
- 300 * packet_loss
- 100 * timeout_rate
- 2 * active_downloads
```

当当前节点失败时，不应直接更新文件状态为失败，而是：

```text
mark replica/node failure metric
  -> try next ready replica
  -> if checksum failed, mark replica corrupt
  -> if all replicas failed, mark chunk/file repair_needed or failed
```

## 10. 副本放置策略

默认副本因子：

| 文件类型 | replication_factor |
| --- | --- |
| 临时文件，低价值 | 1 |
| 普通临时文件 | 2 |
| 重要文件 | 3 |

放置原则：

- 优先跨 zone。
- 避免同一物理磁盘或同一故障域。
- 不把新副本放到 `degraded`、`readonly`、`draining` 或 `offline` 节点。
- 弱网高丢包节点不作为复制目标，但可以作为最后下载备选。

## 11. 传输协议演进

第一阶段使用 HTTP/1.1 或 HTTP/2：

- 实现成本低。
- 与当前 Axum/reqwest 技术栈兼容。
- 易于调试和部署。

第二阶段评估 HTTP/3/QUIC：

- 降低高丢包场景下的队头阻塞。
- 更适合移动网络和跨地域弱网。
- 需要额外验证 Rust 服务端生态和部署代理链路支持情况。

不建议第一阶段自研二进制协议。优先把 chunk、校验、重试、断点和节点切换做好。

## 12. 关键指标

建议采集：

- 上传成功率。
- 平均恢复耗时。
- 重传数据量。
- 下载首字节时间。
- chunk put/get 成功率。
- chunk put/get p95 延迟。
- 副本修复完成时间。
- 节点 timeout rate。
- 节点 packet loss 估计。
