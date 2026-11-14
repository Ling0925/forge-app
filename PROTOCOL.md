# forge-protocol

Command 只返回 `Accepted{id}` 或 `Rejected{code, message}`；结果与后续进展通过带全局单调 `seq` 的 `EventEnvelope` 发布。serde 名称使用 snake_case。

P0 已实现类型、命令应答通道、seq 分配和事件广播。调用方循环驱动 `Runtime::dispatch_next()`；目前所有业务 Command 均返回 `Rejected{code: not_implemented}`，不发布成功事件。返回 `false` 表示命令通道关闭。`next_command()` 保留为后续业务处理和通道测试的底层入口。

## Command 与实现阶段

下表阶段指业务处理；所有类型均已在 P0 提供。

| Command | 阶段 |
|---|---|
| `AddWorkspace{path}` | P1 |
| `RemoveWorkspace{workspace}` | P1 |
| `TestProviderKey{provider}` | P1 |
| `CreateSession{workspace, title, model, permission_mode}` | P1 |
| `RenameSession{session, title}` | P1 |
| `ArchiveSession{session, archived}` | P1 |
| `DeleteSession{session}` | P1 |
| `LoadHistory{session, before_seq}` | P1 |
| `Send{session, input_id, content}` | P1 单轮发送；P3 忙时排队 |
| `Steer{run, steer_id, content}` | P3 |
| `EditQueueItem{item, content}` | P3 |
| `RemoveQueueItem{item}` | P3 |
| `ReorderQueueItem{item, before}` | P3 |
| `ResolveUncertain{item, action}` | P3 |
| `PauseQueue{session}` | P3 |
| `ResumeQueue{session}` | P3 |
| `CancelRun{run}` | P2 |
| `RetryRun{run}` | P3 |
| `RetryNow{run}` | P3 |
| `DecideApproval{approval, decision, scope}` | P2 |
| `SetPermissionMode{session, mode}` | P2 |
| `PreviewRestore{checkpoint}` | P2 |
| `RestoreCheckpoint{checkpoint, overwrite}` | P2 |
| `UpdateSettings{settings}` | P1 |
| `Shutdown` | P2 取消编排；P3 接入 R9 恢复 |

## Event

| 分组 | Event |
|---|---|
| 工作区 | `WorkspaceAdded{workspace}`、`WorkspaceRemoved{workspace}` |
| Key 校验 | `ProviderKeyTested{provider, ok, error_kind}` |
| 会话 | `SessionCreated`、`SessionUpdated`、`SessionArchived`、`SessionDeleted` |
| 队列 | `QueueItemAdded`、`QueueItemUpdated`、`QueueItemRemoved`、`QueuePaused`、`QueueResumed` |
| 运行 | `RunStarted`、`RunStateChanged`、`RunFinished` |
| 插话 | `SteerApplied{run, steer_id}` |
| 生成 | `TextDelta`、`ReasoningDelta`、`ProviderAttempt`、`ProviderRetrying`、`GenerationReset` |
| 工具 | `ToolCallProposed`、`ToolCallStateChanged`、`ToolOutputChunk` |
| 审批 | `ApprovalRequested`、`ApprovalResolved` |
| 上下文 | `ContextUsage`、`ContextCompacted` |
| 检查点 | `CheckpointCreated`、`RestorePreviewed{checkpoint, files}`、`CheckpointRestored{checkpoint, restored, skipped, failed}` |
| 用量与通知 | `UsageRecorded`、`Notice` |
| 退出 | `ShutdownComplete{stopped, unfinished}` |

## 补充约定

- `ProviderKeyTested` 成功时 `error_kind = null`；失败分类为 `network / rate_limited / auth / quota / server / configuration / provider`。请求只有 provider ID，Key 仍由 Keychain 获取。
- `LoadHistory.before_seq: Option<u64>`：有值时只读取该 seq 之前的历史；`null` 从最新历史开始。
- `PreviewRestore` 的结果使用 `RestorePreviewed`，遵守 Command 只应答接受/拒绝的约定。`files` 每项为 `RestorePreviewFile{path, status}`，状态为 `clean / modified_externally / deleted_externally`。
- `RestoreCheckpoint.overwrite: Vec<PathBuf>` 指定允许覆盖冲突的路径，替换原 `overwrite_paths`，无旧字段别名。`CheckpointRestored.restored/skipped` 是路径列表，`failed` 是 `RestoreFailure{path, reason}` 列表。
- `Send.input_id`、`Steer.steer_id` 由 UI 生成。终态后插话转为队列项时，`QueueItem.id` 的字符串值必须等于 `steer_id`；`SteerApplied` 只在插话实际纳入 Provider 请求后发布。
- `ReorderQueueItem.before = null` 表示移动到队尾；有值表示移动到指定项之前。
- `ResolveUncertain.action` 为 `requeue / discard`；`RetryNow` 用于退避等待中的立即重试。

## Shutdown

P0 仅提供 `Shutdown` / `ShutdownComplete` 类型，`dispatch_next()` 返回 `not_implemented`。当前 Runtime 尚无活动 Run 管理和 R4 的 Run 取消入口，只有底层命令进程组取消；三秒等待编排留待 P2。

实现时，UI 在用户确认退出后发送 `Shutdown`。Core 对全部活动 Run 复用 R4 取消路径，共用注入 Clock 的三秒等待窗口；结束后发布 `ShutdownComplete`，已结束的归入 `stopped`，仍未结束的归入 `unfinished`。后者保持非终态，下次启动由 R9 标记 `interrupted`。
