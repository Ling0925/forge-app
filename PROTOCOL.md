# forge-protocol

Command 只返回 `Accepted{id}` 或 `Rejected{code, message}`；结果与后续进展通过带进程内全局严格单调 `seq` 的 `EventEnvelope` 发布。serde 名称使用 snake_case。

P1 已实现下表标记的业务命令。调用方循环驱动 `Runtime::dispatch_next()`，同时处理命令与 Provider 任务发回的流事件；Provider 请求不会阻塞命令循环。所有数据库写入和事件广播都在此循环完成。状态与持久事件在同一事务提交后才广播；文本/推理增量只在活动 Run 内存中积累，成功终态才保存助手消息。返回 `false` 表示命令通道关闭。`Runtime::new` 使用调用方提供的 `initial_settings` 初始化空库，已有设置优先，不内置模型名。

## Command 与实现阶段

下表阶段指业务处理；P2/P3 命令目前返回 `Rejected{code: not_implemented}`。

| Command | 阶段 |
|---|---|
| `AddWorkspace{path}` | P1 已实现 |
| `RemoveWorkspace{workspace}` | P1 已实现 |
| `TestProviderKey{provider}` | P1 已实现 |
| `SetProviderKey{provider, key}` | P1 已实现 |
| `GetSnapshot` | P1 已实现 |
| `CreateSession{workspace, title, model, permission_mode}` | P1 已实现 |
| `RenameSession{session, title}` | P1 已实现 |
| `ArchiveSession{session, archived}` | P1 已实现 |
| `DeleteSession{session}` | P1 已实现 |
| `LoadHistory{session, before, limit}` | P1 已实现 |
| `Send{session, input_id, content}` | P1 已实现发送；忙时拒绝 invalid_state，P3 排队；Provider 响应内的只读工具调用按末节执行并续跑 |
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
| `UpdateSettings{settings}` | P1 已实现 |
| `Shutdown` | P2 取消编排；P3 接入 R9 恢复 |

## Event

| 分组 | Event |
|---|---|
| 工作区 | `WorkspaceAdded{workspace}`、`WorkspaceRemoved{workspace}` |
| Key | `ProviderKeyTested{provider, ok, error_kind}`、`ProviderKeyConfigured{provider, key_status}` |
| 设置 | `SettingsUpdated{settings}` |
| 历史 | `HistoryLoaded{session, messages, next_before, runs}` |
| 快照 | `Snapshot{sessions, workspaces, settings, providers, active_runs, last_seq}` |
| 会话 | `SessionCreated`、`SessionUpdated`、`SessionArchived`、`SessionDeleted` |
| 队列 | `QueueItemAdded`、`QueueItemUpdated`、`QueueItemRemoved`、`QueuePaused`、`QueueResumed` |
| 运行 | `RunStarted{run, input_id}`、`RunStateChanged`、`RunFinished{run, outcome, finished_at}` |
| 插话 | `SteerApplied{run, steer_id}` |
| 生成 | `TextDelta`、`ReasoningDelta`、`ProviderAttempt`、`ProviderRetrying`、`GenerationReset` |
| 工具 | `ToolCallProposed`、`ToolCallStateChanged`、`ToolOutputChunk` |
| 审批 | `ApprovalRequested`、`ApprovalResolved` |
| 上下文 | `ContextUsage`、`ContextCompacted` |
| 检查点 | `CheckpointCreated`、`RestorePreviewed{checkpoint, files}`、`CheckpointRestored{checkpoint, restored, skipped, failed}` |
| 用量与通知 | `UsageRecorded`、`Notice` |
| 退出 | `ShutdownComplete{stopped, unfinished}` |

## 补充约定

- `TextDelta`、`ReasoningDelta`、`HistoryLoaded`、`Snapshot`、`ProviderKeyTested` 是临时事件：在运行循环内分配 seq 并广播，不写入 `events` 表，也不启动 SQLite 事务。状态变更与对应持久事件仍在同一事务提交后广播。
- seq 在进程内严格单调递增。重启后从 `max(调用方 last_seq, events 表中最后持久化的 seq)` 之后继续；尾部临时事件的 seq 不持久化，重启后可能复用。客户端在 Core 重启后必须重新获取 Snapshot，并用其 last_seq 重置游标，不能沿用重启前临时事件的游标。
- `ProviderKeyTested` 成功时 `error_kind = null`；失败分类为 `network / rate_limited / auth / quota / server / configuration / provider`。请求只有 provider ID，Key 仍由 Keychain 获取。
- `LoadHistory.before: Option<u64>` 是持久化的 `Message.ordinal`，不是 Event seq。`limit: u32` 指定页大小（0 返回空页）。选择 ordinal < before 的最新 limit 条，None 从最新开始；返回顺序为旧→新。仅当仍有更旧消息时，`next_before` 为本页最小 ordinal，否则为 None。ordinal 单调递增，删除消息后不复用。
- `HistoryLoaded.runs: Vec<Run>` 包含本页 messages 引用的全部 Run，按 Run ID 排序、去重，不包含其他页独有的 Run。复用现有 `Run.outcome: Option<RunOutcome>` 和 `Run.finished_at: Option<u64>`；运行中二者为 None，终态二者有值。input_id 仍在用户 Message 上。旧事件缺少 runs 时反序列化为空列表，空列表序列化时省略。
- `GetSnapshot` 在运行循环内构造，`last_seq` 等于承载 Snapshot 的 envelope.seq。消费端以快照替换会话/工作区/设置与活动 Run 状态，此后只应用 seq > last_seq 的事件；seq 断档时重新请求快照。历史消息另用 LoadHistory 加载。
- `Session.last_finished_run: Option<FinishedRun>` 随 Snapshot.sessions 和会话事件返回，`FinishedRun{run: RunId, input_id: Option<InputId>, outcome: RunOutcome, started_at: u64, finished_at: u64}` 表示该会话最近完成的 Run；按 finished_at 取最新，同一毫秒按 Run 入库顺序取较新者。字段从持久化 Run 和用户 Message 读取，无已完成 Run 时为 None；新 Run 运行中仍保留上次终态。started_at / finished_at 为 UTC Unix 毫秒，分别取自同一 Run 的持久化字段。旧 Session 缺少该字段时反序列化为 None，None 序列化时省略。丢失 RunFinished 或重启后，可通过 GetSnapshot 恢复最近终态，通过 LoadHistory 恢复对应历史 Run 的终态。
- `providers` 为设置中各 Provider 的 `ProviderKeyStatus{provider, key_status}`；`active_runs` 为 `ActiveRun{run, input_id, text, reasoning}`，包含当前已发布的所有文本和推理增量。终态 Run 从 active_runs 移除。snapshot 不包含密钥。
- `KeyStatus` 为 `configured / missing / unavailable`，序列化为 snake_case 字符串。`ProviderKeyStatus.key_status` 替换原 `key_configured: bool`，`ProviderKeyConfigured.key_status` 替换原 `configured: bool`，属于破坏性变更，不保留旧字段。configured 表示已配置非空 Key，UI 显示已配置；missing 表示未配置或空字符串，UI 沿用现有无 Key 行为；unavailable 表示 SecretStore 读取失败，仅将对应 Provider 标记为不可用，GetSnapshot 仍正常返回其他状态。UI 显示“无法读取钥匙串”及重试按钮；重试重新发送 GetSnapshot。
- `SetProviderKey.key: Option<String>`：Some 保存，None 删除。Keychain 的 service 为 `dev.forge.app`、account 为 provider ID；`keychain_reference` 不参与索引。空字符串等同未配置。`ProviderKeyConfigured` 在 SecretStore 写入成功后发布；设置和事件只包含配置状态，密钥不写 SQLite、不进事件或错误。
- `Settings.ui_prefs: BTreeMap<String, String>` 为 UI 定义的透明字典，Core 原样存取。`ProviderSettings.endpoint` 覆盖 API 根地址；None 使用公共地址，空/无效地址为 configuration。`ProviderSettings.max_output_tokens: Option<u64>` 由调用方指定：Anthropic 要求提供并发送为 max_tokens；OpenAI 有值时发送 max_completion_tokens，无值时省略。TestProviderKey 使用所选 Provider 的已配置模型（全局选择，或该 Provider 的会话模型），发送一个输出 token 的最小请求，不写会话历史。
- Session.created_at / updated_at、Run.started_at / finished_at 均为注入 Clock 的 UTC Unix 毫秒，持久化；finished_at 在非终态为 None。Rename/Archive/运行状态改变同步 updated_at。运行中的会话不可删除；仍有会话的工作区不可移除。
- `AddWorkspace` 使用 canonicalize 后的根路径去重，名称取最后一段；重复添加返回同一 ID，不重复发布 WorkspaceAdded。
- `ContextUsage.context_remaining_estimate`：上下文上限减去下一次请求的估算用量，约数。
- `PreviewRestore` 的结果使用 `RestorePreviewed`，遵守 Command 只应答接受/拒绝的约定。`files` 每项为 `RestorePreviewFile{path, status}`，状态为 `clean / modified_externally / deleted_externally`。
- `RestoreCheckpoint.overwrite: Vec<PathBuf>` 指定允许覆盖冲突的路径，替换原 `overwrite_paths`，无旧字段别名。`CheckpointRestored.restored/skipped` 是路径列表，`failed` 是 `RestoreFailure{path, reason}` 列表。
- P1 `Send` 在一个事务中保存 Run、Turn、带唯一 input_id 的用户 Message 和会话 Running 状态；提交后先应答 Accepted{id: input_id}，再广播 RunStarted 与 SessionUpdated。空闲发送不发布 QueueItem 事件。同一 input_id、同一会话及相同 Content 返回原 Accepted，无新事件或请求；内容或会话不同返回 conflict，重启后仍去重。忙时新 input_id 返回 invalid_state。
- 成功结束时同一事务保存助手 Message、Run.outcome/usage/finished_at、UsageRecord 和 Idle 会话，再广播 UsageRecorded、SessionUpdated、RunFinished。Provider 失败不保存部分助手消息，不自动重试。
- `RunFinished.finished_at: u64` 为终态的 UTC Unix 毫秒，与同次写入的 `Run.finished_at` 相同；正常完成、Provider 失败、存储故障和启动恢复均使用各自终态处理时的同一个时间值。`RunFinished.finished_at` 与 `FinishedRun.started_at` 是向后兼容的新增字段；旧事件或 FinishedRun 缺少对应字段时反序列化为 0，表示旧数据未提供时间，新增输出始终携带实际值。存储故障时仍遵守下述尽力落盘例外。
- `Runtime::new` 在接受命令前恢复数据库遗留的 Running Run：将 state 置为 Failed，outcome 置为 `Failed{kind: interrupted, message: "上次异常退出时中断", http_status: None, retry_after_ms: None, retryable: true}`，finished_at 使用本次启动的注入 Clock 当前时间；所属 Session 置为 Idle，updated_at 同步该时间。每个 Run 的终态、会话状态及 SessionUpdated / RunFinished 事件在同一事务持久化，seq 接续已有事件。恢复后 active_runs 不包含这些 Run，Send/DeleteSession 可正常执行；再次启动不会改写已恢复的终态。此规则覆盖 send.after_commit 和 run.before_final_commit 异常退出；恢复存储失败仍由 Runtime::new 返回错误。
- `Run.outcome: Option<RunOutcome>` 取代 terminal_reason，`RunFinished` 不再有 reason。RunOutcome 使用 `type` 标签：`{type:succeeded}`、`{type:cancelled, source:user|app_quit}`、`{type:failed, kind, message, http_status, retry_after_ms, retryable}`。Failed.kind 为 network / rate_limited / auth / quota / server / context_too_long / interrupted，或 `Unknown{source: provider|storage}`。Unknown 的 JSON 为 `{"unknown":{"source":"provider"}}` 或 `{"unknown":{"source":"storage"}}`，其余 kind 仍为 snake_case 字符串；R9 的 interrupted 使用 Failed，非独立 outcome。
- `ProviderError.code: Option<String>` 来自结构化错误：Anthropic 的 HTTP 错误体与流式 error 事件读取 `error.type`；OpenAI 先读取 `error.code`，缺失时读取 `error.type`。code 与 message 都进行密钥脱敏。Core 在发布和持久化 RunOutcome::Failed 前，还对 message 执行请求头脱敏：以真实换行 LF、CRLF 及字面量两字符 `\n`、四字符 `\r\n` 分段，保留分隔符原样。每段中，凭据头名称不区分大小写，可带引号或反斜杠转义引号，名称后允许空白和闭引号，再接 `:` 或 `=`；从名称的首字符到该段末尾替换为 `[已移除请求头]`，名称之前的内容（包括开引号）、其他段和分隔符均保留。不整段删除，也不添加行数标记。名称是仅含字母、数字、`-`、`_` 的单个词：authorization、proxy-authorization、cookie、set-cookie，或任何以 api-key、api_key、apikey、token、secret 结尾的名称（如 x-auth-token、client_secret）。`invalid api key: please check settings` 保持不变；单行 `Authorization: Bearer …` 的结果恰为非空文本 `[已移除请求头]`。
- Run 错误按下表分类。唯一允许检查 message 的情况是 Anthropic 的 invalid_request_error，且 message 含 `prompt is too long`，映射为 context_too_long。ProviderKeyTested 的配置校验仍返回 configuration。
- http_status 保留 HTTP 错误响应状态；成功响应流中的 API error 使用 ErrorKind::Api，http_status 为 None。ProviderError::retryable 不搜索 message：HTTP 仅 429/5xx 为 true；Network/Disconnected/InvalidResponse 为 true；Api 的 authentication_error / permission_error / invalid_api_key / insufficient_quota / billing_error / context_length_exceeded 为 false，其余 code（包括缺失）为 true。Core 将 Anthropic 上述上下文超限特例的 retryable 设为 false。Retry-After 支持秒与 HTTP-date，日期以注入 Clock 转为非负剩余毫秒，不截断。retryable 不表示 P1 会自动重试。
- 运行中的存储错误终止该 Run，移除 active_runs 中的缓存，并尽力持久化 Failed 终态与 Idle 会话；若终态写入失败，仍尝试单独保存 Idle 会话。不循环重试；恢复写入仍失败时也广播 SessionUpdated（能读取会话时）与 RunFinished，seq 继续递增，运行循环继续处理命令。这是先落盘再广播的存储故障例外；失败详情保留存储错误文本，并执行相同的密钥脱敏和请求头分段替换。启动时 Runtime::new / Store::open 的存储错误仍返回调用方，不生成 RunOutcome。
- `Send.input_id`、`Steer.steer_id` 由 UI 生成。终态后插话转为队列项时，`QueueItem.id` 的字符串值必须等于 `steer_id`；`SteerApplied` 只在插话实际纳入 Provider 请求后发布。
- `ReorderQueueItem.before = null` 表示移动到队尾；有值表示移动到指定项之前。
- `ResolveUncertain.action` 为 `requeue / discard`；`RetryNow` 用于退避等待中的立即重试。

下表是 Core → UI 错误映射的唯一依据（对应 UI 规格 §6.7）。分类依次匹配状态码与结构化 code；HTTP 的 retryable 始终由状态码决定（429/5xx 为 true，其余为 false），即使 code 改变分类。表中 code 对应的 retryable 指无 HTTP 状态的流内错误。

| 错误来源 | Failed.kind | UI 卡片 | retryable |
|---|---|---|---|
| HTTP 401/403；authentication_error / permission_error / invalid_api_key | auth | API Key 无效或没有权限；打开设置 | false |
| HTTP 402；insufficient_quota / billing_error | quota | 账户额度不足 | false |
| HTTP 429；rate_limit_error / rate_limit_exceeded | rate_limited | 请求太频繁；retry_after_ms > 60000 时为“服务要求等待”长等待卡片 | true |
| HTTP 5xx；overloaded_error / api_error / server_error | server | 模型服务出错 | true |
| context_length_exceeded；Anthropic invalid_request_error 且 message 含 `prompt is too long` | context_too_long | 对话太长 | false |
| Network / Disconnected | network | 连不上模型服务 | true |
| Configuration（缺少 Key、无效 endpoint 等） | auth | API Key 无效或没有权限；打开设置 | false |
| 无 HTTP 状态的 InvalidResponse（响应格式错误、流解析失败）；未被上述 code 规则分类的 Api 错误 | server | 模型服务出错；重试 | true |
| 其余 Provider 错误 | unknown{source: provider} | 运行失败 | false |
| 运行中的存储错误 | unknown{source: storage} | 保存失败；运行结果没能写入本地数据，详情里有原因 | false |
| 启动恢复发现 Running Run | interrupted | 上次异常退出时中断 | true（由用户发起重试） |

## Shutdown

P1 保留 `Shutdown` / `ShutdownComplete` 类型，业务处理返回 `not_implemented`。R4 Run 取消入口与三秒等待编排留待 P2，当前只有底层命令进程组取消。

实现时，UI 在用户确认退出后发送 `Shutdown`。Core 对全部活动 Run 复用 R4 取消路径，共用注入 Clock 的三秒等待窗口；结束后发布 `ShutdownComplete`，已结束的归入 `stopped`，仍未结束的归入 `unfinished`。后者保持非终态，下次启动由 R9 标记 `interrupted`。

## UI-REF-06 文件改动行数

`forge-tools` 的 `write_file`、`edit_file` 返回 `FileChange { path, before_sha256, after_sha256, lines_added, lines_removed }`：`path` 为解析后的工作区相对路径，前后哈希为内容的 SHA-256，新文件的 `before_sha256` 为 `None`；两个行数字段均为 `u32`。

UI Inspector diff 采用相同计数规则：对前后原始字节使用 `similar::TextDiff::from_lines` 的 Myers 行差异，`Insert` 每行计入 `lines_added`，`Delete` 每行计入 `lines_removed`。分行保留行结束符（包括 `\n` 和完整的 `\r\n`）；末行即使没有换行符也算一行；仅末尾换行符变化计为删除一行、增加一行。非 UTF-8 内容直接按字节比较，不做有损转换。新文件删除行数为 0，增加行数为新内容行数（空文件为 0）；内容未变时两项均为 0。

忽略规则与隐藏条目过滤仅用于非递归的 `list_dir` 及 `glob`、`grep`：无需 Git 仓库，仍应用目录树内的 `.gitignore`、`.ignore` 和存在时的 `.git/info/exclude`，不应用工作区外父目录或全局忽略规则；按名称调用 `read_file`、`write_file`、`edit_file` 不受这些过滤规则影响。

## P2 只读工具循环

- `Send` 发起的 `ProviderRequest.tools` 只发布本切片实际支持的工具：`read_file`、`list_dir`、`glob`、`grep`，四个只读工具 `has_side_effects=false`，不需要审批。`write_file`、`edit_file`、`command` 等副作用工具不发布也不可执行；模型仍调用时按 `failed` 工具结果回传“该工具在此构建不可用”。`has_side_effects` 描述工具本身的性质而非本构建是否执行：被拒绝的副作用工具与未知名称均记为 `true`，以便未来审批门控不会把未验证的调用当成无副作用。参数为 JSON Schema：`read_file.lines` 是 `[start, end]` 一元起始闭区间；`glob`/`grep` 的 `path` 省略时默认工作区根。
- 流式工具调用按 `ToolCallId` 组装 `ToolCallStarted` 与多个 `ToolArgumentsDelta` 的 `partial_json` 增量，顺序与内容原样拼接；同一响应中重复 call id、复用任一已持久化 call id（本会话早期轮次或更早 Run 留下的 `tool_calls` 行）、或对未开始 call 的参数增量，按 `invalid_response`（Failed.kind: server）终止该 Run，已有调用行不被改写。流中途出错时该响应的未完成调用整体丢弃，不产生 `ToolCallProposed`。无参数增量时 `arguments_json` 记为 `{}`。Run 到达终态后，Provider 流中迟到的事件（增量、Usage、错误、Finished）被丢弃，不再影响该 Run。
- 一次 Provider 响应的全部调用先各自提交并广播 `ToolCallProposed`（`state: proposed`，含组装后的 `arguments_json`），随后按流式顺序逐个执行：执行前发布 `ToolCallStateChanged{running}`，结束发布终态 `succeeded`/`failed`/`cancelled`，`result` 为结果文本或错误文本。`ToolCallProposed` 与 `ToolCallStateChanged` 是持久事件，与其他状态事件在同一事务提交后广播。产生这些调用的 assistant `Message` 和全部 `tool_calls` 行与提案事件在同一事务写入；首个调用的 `running` 状态也在其中。每个调用结束时，更新后的 `tool_calls` 行与 `role: tool`、`tool_call_id` 指向该调用的结果 `Message` 随终态事件同事务写入。
- `ProviderMessage` 增加 `tool_calls: Vec<ProviderToolCall{id, name, arguments_json}>`（assistant 消息携带其发起的调用）与 `is_error: bool`（tool 消息标记失败结果）。续跑前 Core 按 `messages.ordinal` 重建会话消息：每条 `role: tool` 结果挂到最近一条 assistant 消息上，同时把对应 `tool_calls` 行追加为该 assistant 的 `tool_calls`；没有结果消息的调用（中断/取消）从续跑消息中省略，不发送未配对调用。HTTP 编码按 Provider 分形：Anthropic 把 assistant 写为 `text`+`tool_use` 内容块、把结果写为 `user` 消息里的 `tool_result` 块（失败时 `is_error: true`，连续结果合并入同一 user 消息）；OpenAI 把调用写为 assistant `tool_calls`（`type: function`，`arguments` 为原始 JSON 字符串）、结果写为 `role: tool` 消息并携带 `tool_call_id`。call id 全程原样透传。Anthropic `tool_use.input` 必须是 JSON 对象：参数解析为对象时原样写入；解析不出对象（截断 JSON、或合法但非对象的值）时把原始参数文本放进 `{"_forge_malformed_arguments": <原始文本>}`，不静默改写成空对象，配对 `tool_result` 已带 `is_error` 错误文本。某次响应没有工具调用时按既有终态处理：成功时保存的 assistant 消息只含该次响应的文本。`Run.usage` 累计该 Run 全部 Provider 请求的用量（`requests` 为请求次数），`UsageRecord` 在终态记录合计值。
- `read_file` 结果即文件文本；`list_dir` 每行一个条目、目录以 `/` 结尾；`glob` 每行一个相对路径；`grep` 每行 `path:line:text`。参数不是合法 JSON、缺字段或行区间非法 → `failed`（InvalidArguments）；执行中的 IO/规则错误同样 → `failed`，错误文本作为工具结果回传给模型。
- `ToolError::Cancelled` 映射为 `cancelled`，Run 终态为 `Cancelled{source: user}`。本切片没有取消入口：`CancelRun`、`DecideApproval`、`SetPermissionMode`、`Shutdown` 仍返回 `not_implemented`；glob/grep 已接入 `CancellationToken`，但没有路径触发取消，该映射当前不可达。
- UI 可回放范围：assistant 文本经 `TextDelta`（以及 `Snapshot.active_runs`、`LoadHistory` 的 assistant/user 消息）回放；工具生命周期经持久事件 `ToolCallProposed`/`ToolCallStateChanged`（含终态 `result`）回放。当前 `forge-state` 同步跳过 `MessageRole::Tool` 消息行且不渲染 `tool_calls` 行——结果消息与调用参数只存在于持久行与事件中，UI 不把工具结果消息显示为独立转录条目；`approvals` 表不写行，`AwaitingApproval`/`Approved`/`Denied` 等审批态不使用；`ToolOutputChunk` 不发布（本切片工具无流式输出）。
