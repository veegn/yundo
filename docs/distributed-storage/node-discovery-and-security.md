# 节点发现与安全设计

本文定义 Storage Node 注册、心跳、服务发现视图、状态收敛、缓存失效和内部安全边界。

## 1. 服务发现模式

第一阶段采用“控制平面内建注册表 + 存储节点主动注册/心跳”的模式，不额外引入 Consul、etcd 或 Kubernetes Service Discovery。

原因：

- 当前系统规模和复杂度可控。
- 节点注册信息需要与容量、RTT、丢包率、读写负载、副本状态绑定。
- 业务调度状态直接落入业务数据库更利于调度和审计。

后续扩展：

- Kubernetes：通过 Service、EndpointSlice 发现节点基础地址，业务心跳仍写入 DB。
- Consul/etcd：用于节点基础服务发现，Yundo DB 仍保存容量、负载、副本和调度状态。
- 静态配置：适合小规模私有部署，启动时预置节点列表，节点仍需心跳确认可用。

## 2. 节点注册

Storage Node 启动后向 API 注册：

```text
POST /api/storage/nodes/register
```

请求：

```json
{
  "node_id": "node-a",
  "name": "storage-node-a",
  "endpoint": "https://node-a.internal.example",
  "zone": "cn-east-1a",
  "capacity_bytes": 1099511627776,
  "storage_version": "0.1.0",
  "features": ["chunk-read", "chunk-write", "checksum", "replication"],
  "public_download": false
}
```

响应：

```json
{
  "registered": true,
  "node_id": "node-a",
  "heartbeat_interval_secs": 30,
  "heartbeat_ttl_secs": 90,
  "assigned_status": "registered",
  "server_time": "2026-05-24T14:30:00Z"
}
```

注册规则：

- `node_id` 必须稳定，节点重启后保持不变。
- `node_id` 必须与 token 绑定，避免节点冒充。
- `endpoint` 必须是 API 节点可以访问的内部地址。
- 已存在 `node_id` 时执行幂等更新，只更新 endpoint、zone、capacity、features、version 等可变字段。
- 注册成功不代表立即参与调度，调度器需要等待至少一次成功心跳。
- 如果节点曾处于 `draining` 或 `readonly`，重新注册不能自动恢复为 `active`，需要管理员显式操作或恢复策略确认。
- endpoint 变更应记录审计日志；生产环境建议要求管理员确认或更高权限 token。

## 3. 心跳与租约

注册只建立节点身份，可用性由心跳租约决定。

```text
POST /api/storage/nodes/:id/heartbeat
```

建议：

- 心跳间隔：30 秒。
- 心跳 TTL：90 秒。
- 判断依据：`now - last_heartbeat_at <= heartbeat_ttl`。

心跳应携带：

- 磁盘总量和已用量。
- 活跃上传、下载、复制任务数。
- 本地磁盘健康状态。
- API 到节点或节点到 API 的 RTT。
- 最近窗口内失败率和超时率。
- 可选丢包率估计。

心跳只提供观测数据，最终状态由控制平面结合管理员状态和调度策略计算。

## 4. 服务发现视图

控制平面对内部调度器、repair worker 或管理端提供只读发现视图：

```text
GET /api/storage/nodes
```

响应：

```json
{
  "nodes": [
    {
      "id": "node-a",
      "endpoint": "https://node-a.internal.example",
      "zone": "cn-east-1a",
      "status": "active",
      "live": true,
      "free_bytes": 976128930611,
      "active_uploads": 2,
      "active_downloads": 8,
      "active_replications": 1,
      "avg_rtt_ms": 180,
      "p95_rtt_ms": 420,
      "packet_loss": 0.08,
      "timeout_rate": 0.03,
      "features": ["chunk-read", "chunk-write", "checksum", "replication"],
      "last_heartbeat_at": "2026-05-24T14:30:00Z"
    }
  ]
}
```

调度器不应直接使用所有已注册节点，而应使用计算后的发现视图：

```text
registered nodes
  -> filter heartbeat expired
  -> filter admin disabled/draining/readonly
  -> filter missing required features
  -> filter insufficient capacity
  -> filter unhealthy disk
  -> score by zone, RTT, packet loss, timeout rate, load, free ratio
```

## 5. 节点状态收敛

状态转换建议：

```text
register success
  -> registered
first heartbeat success
  -> active or readonly
heartbeat expired
  -> offline
heartbeat restored
  -> degraded
health stable for N heartbeats
  -> active
admin drain
  -> draining
admin disable
  -> readonly or offline
```

`offline -> active` 不建议立即完成。节点恢复后应先进入 `degraded`，后台执行轻量校验：

- 检查节点存储目录可读写。
- 抽样校验部分 chunk。
- 对比节点已用空间与元数据估算。
- 确认失败率低于阈值。

连续多个心跳窗口稳定后，再恢复为 `active`。

## 6. 本地缓存与失效

API 调度器可以缓存服务发现结果，避免每次上传或下载都查询 DB。

建议：

- 缓存 TTL：5 秒。
- 节点下线、管理员禁用、draining 操作后立即主动失效。
- 上传会话创建时固定首选节点列表，单个 chunk 失败时重新读取发现视图。
- 下载每个 chunk 可按缓存视图选择副本，失败时强制刷新节点视图后重试。

## 7. 弱网发现策略

高延迟、高丢包环境中，不能只用心跳是否成功判断节点质量。应使用滑动窗口指标降低抖动影响。

建议每个节点维护最近 5 分钟窗口：

```text
heartbeat_success_rate
chunk_put_success_rate
chunk_get_success_rate
avg_rtt_ms
p95_rtt_ms
timeout_rate
packet_loss
```

调度规则：

- `heartbeat_success_rate < 80%`：标记 `degraded`。
- `timeout_rate > 20%`：不再接受新上传，只保留下载备选。
- `packet_loss > 10%`：降低上传并发上限。
- `packet_loss > 20%`：仅作为最后下载备选，不参与副本复制目标。
- `p95_rtt_ms` 明显高于集群均值时，降低调度分数但不直接剔除。

弱网下的节点发现应偏向稳定性，而不是追求瞬时低延迟，避免节点在 `active` 和 `offline` 之间频繁抖动。

## 8. 内部认证

最低要求：

- 注册、心跳、内部 chunk 接口均要求 `Authorization: Bearer <internal-token>`。
- 每个节点使用独立 token。
- `node_id` 与 token 绑定。
- token 权限区分注册、心跳、读 chunk、写 chunk、删 chunk、复制 chunk。
- 关键操作记录审计日志，包括注册、状态变更、draining、删除节点。

推荐增强：

- 内部接口使用 mTLS。
- 心跳携带 nonce 或时间戳签名，降低重放风险。
- endpoint 变更需要管理员确认或更严格 token 权限。
- 内部 URL 使用短有效期签名。

## 9. endpoint SSRF 防护

节点注册 endpoint 是高风险输入。控制平面如果主动探测 endpoint，可能被诱导访问本地敏感地址、云厂商 metadata service 或内网管理服务。

强制要求：

- endpoint scheme 只允许 `https`，开发环境可显式允许 `http`。
- host 必须满足允许网段或域名白名单。
- 禁止 loopback 地址，例如 `127.0.0.0/8`、`::1`。
- 禁止 link-local 地址，例如 `169.254.0.0/16`、`fe80::/10`。
- 禁止云厂商 metadata service 地址，除非部署环境显式允许且经过隔离。
- 禁止注册到任意公网地址，除非管理员配置公网节点白名单。
- 禁止 URL 中携带用户名和密码。
- 限制端口范围，只允许配置中的内部服务端口。

API 主动探测规则：

- 只请求固定 path，例如 `/internal/healthz`。
- 使用短连接超时和总超时。
- 不跟随重定向。
- 不把用户可控 header 透传到探测请求。
- 探测响应必须包含节点身份声明，并与注册的 `node_id` 匹配。

## 10. object key 安全

Storage Node 不得把外部传入的 object key 直接拼接成文件系统路径。

要求：

- object key 必须匹配受控格式，例如 `files/{file_id}/chunks/{chunk_index}` 或 `uploads/{upload_id}/chunks/{chunk_index}`。
- `file_id`、`upload_id`、`chunk_index` 必须分别校验字符集和长度。
- 禁止 `..`、空路径段、反斜杠、控制字符和 URL 二次编码绕过。
- 最终落盘路径应由服务端根据解析后的结构生成。
- 读取、写入、删除前确认目标路径位于 Storage Node 管理目录内。

## 11. 审计日志

建议记录：

- 节点注册、重新注册、endpoint 变更。
- 节点状态变更。
- 管理员设置 readonly、draining、offline。
- 内部认证失败。
- 删除任务、repair 任务异常。
- endpoint SSRF 校验拒绝事件。

审计日志应包含：

```text
time, actor, node_id, action, result, reason, request_id
```
