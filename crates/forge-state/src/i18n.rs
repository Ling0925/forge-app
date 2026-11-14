//! UI string table. v1 ships Simplified Chinese only (user decision 2026-11-14),
//! but every visible string goes through a key so another locale can be added
//! without touching call sites. Keys follow FORGE_APP_UI_SPEC.md §10; rows that
//! the spec lists as `a / b / c` are split into one key per value.

/// Looks up `key` in the zh-CN table. Unknown keys return the key itself so a
/// missing entry is visible in the UI instead of rendering blank.
pub fn t(key: &str) -> &str {
    lookup(key).unwrap_or(key)
}

/// Returns the zh-CN text for `key`, or `None` when the key is not in the table.
pub fn lookup(key: &str) -> Option<&'static str> {
    ZH_CN
        .binary_search_by(|(k, _)| k.cmp(&key))
        .ok()
        .map(|index| ZH_CN[index].1)
}

/// Looks up `key` and replaces each `{name}` placeholder with its argument.
pub fn tf(key: &str, args: &[(&str, &str)]) -> String {
    let mut text = t(key).to_owned();
    for (name, value) in args {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

/// Formats a duration in seconds as `m:ss` (or `h:mm:ss`), used by run timers.
pub fn format_elapsed(total_secs: u64) -> String {
    let (h, m, s) = (total_secs / 3600, (total_secs % 3600) / 60, total_secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// All keys, sorted by key so lookups can binary-search. A unit test enforces
/// the ordering and uniqueness.
pub static ZH_CN: &[(&str, &str)] = &[
    ("a11y.composer", "消息"),
    ("a11y.composer.hint.idle", "回车发送，Shift 回车换行"),
    (
        "a11y.composer.hint.running",
        "回车加入队列，Option 回车插话",
    ),
    ("a11y.divider", "运行{outcome}"),
    ("a11y.error_card", "运行失败：{title}"),
    ("a11y.reasoning", "思考过程"),
    ("a11y.sidebar", "会话"),
    ("a11y.sidebar.group", "{group}，{n} 个会话"),
    ("a11y.sidebar.resize", "调整侧栏宽度"),
    ("a11y.sidebar.row", "{title}，{status}"),
    (
        "a11y.sidebar.row.approvals",
        "{title}，{status}，{n} 个请求",
    ),
    ("a11y.timeline", "对话记录"),
    ("a11y.timeline.assistant", "Forge：{text}"),
    ("a11y.timeline.user", "你：{text}"),
    ("announce.approval", "需要批准：{action}。按 Command J 查看"),
    ("announce.queue_paused", "队列已暂停：{reason}"),
    ("announce.retrying", "连接中断，正在第 {n} 次重试"),
    ("announce.run.cancelled", "已停止"),
    ("announce.run.failed", "运行失败：{title}"),
    ("announce.run.started", "开始运行"),
    ("announce.run.succeeded", "运行完成，用时 {duration}"),
    ("app.title", "Forge"),
    ("approval.allow", "允许"),
    ("approval.always", "本会话总是允许"),
    ("approval.deny", "拒绝"),
    ("approval.dock", "{n} 个请求等待批准：{action}"),
    ("approval.dock.view", "查看"),
    ("assistant.reasoning", "思考了 {secs} 秒"),
    ("assistant.stopped_suffix", "（已停止）"),
    ("assistant.thinking", "思考中… {elapsed}"),
    (
        "banner.recovery",
        "上次异常退出。已检查 {s} 个会话：{r} 个运行被中断，{u} 条排队消息状态不确定。",
    ),
    ("banner.recovery.view", "查看"),
    ("composer.hint.idle", "↩ 发送 · ⇧↩ 换行"),
    ("composer.hint.running", "↩ 加入队列 · ⌥↩ 插话 · ⌘. 停止"),
    (
        "composer.hint.running_p1",
        "运行结束后才能发送下一条 · ⌘. 停止",
    ),
    ("composer.no_key", "先在设置里添加 API Key"),
    ("composer.not_sent", "没有发送：{reason}"),
    ("composer.open_settings", "打开设置"),
    ("composer.placeholder.idle", "描述你要做的事…"),
    ("composer.send", "发送"),
    ("composer.stop", "停止"),
    ("composer.stopping", "正在停止…"),
    (
        "context.tooltip",
        "上下文已用 {pct}%（{used} / {limit} tokens）",
    ),
    (
        "context.warning",
        "上下文已用 {pct}%，接近上限时会自动压缩较早内容",
    ),
    (
        "delete.confirm.body",
        "会话记录会被删除，已改动的文件不受影响。",
    ),
    ("delete.confirm.cancel", "取消"),
    ("delete.confirm.ok", "删除"),
    ("delete.confirm.title", "删除“{title}”？"),
    ("demo.badge", "演示数据"),
    ("divider.cost", "约 ${cost}"),
    ("divider.elapsed", "用时 {elapsed}"),
    ("divider.tokens", "输入 {input} / 输出 {output} tokens"),
    ("divider.usage_pending", "用量统计中"),
    ("empty.example.1", "解释这个项目的结构"),
    ("empty.example.2", "找出并修复一个失败的测试"),
    ("empty.example.3", "给最近的改动写说明"),
    ("empty.title", "想让 Forge 做什么？"),
    ("error.action.details", "详情"),
    ("error.action.new", "新会话"),
    ("error.action.retry", "重试"),
    ("error.action.retry_wait", "重试（{remaining}）"),
    ("error.action.settings", "打开设置"),
    ("error.action.view_changes", "查看改动"),
    ("error.auth.body", "请在设置里检查 {provider} 的 Key。"),
    ("error.auth.title", "API Key 无效或没有权限"),
    ("error.context.body", "已无法再压缩，建议新开会话继续。"),
    ("error.context.title", "对话太长"),
    ("error.detail.code", "错误码 {code}"),
    ("error.detail.headers_removed", "已移除 {n} 行请求头"),
    ("error.details.hide", "收起详情"),
    ("error.interrupted.body", "这次运行没有完成。"),
    ("error.interrupted.title", "上次异常退出时中断"),
    ("error.network.body", "已自动重试 5 次。请检查网络后重试。"),
    ("error.network.title", "连不上模型服务"),
    ("error.quota.body", "请在服务商后台检查额度，或换一个 Key。"),
    ("error.quota.title", "账户额度不足"),
    ("error.rate_limited.body", "服务要求稍后再试。"),
    ("error.rate_limited.title", "请求太频繁"),
    (
        "error.rate_limited_wait.body",
        "可在 {time} 后重试（还要 {remaining}）。",
    ),
    ("error.rate_limited_wait.title", "服务要求等待"),
    ("error.server.body", "服务暂时不可用，已重试 5 次。"),
    ("error.server.title", "模型服务出错"),
    ("error.unknown.body", "原因见详情。"),
    ("error.unknown.title", "运行失败"),
    ("error_kind.auth", "认证错误"),
    ("error_kind.context", "对话太长"),
    ("error_kind.interrupted", "异常中断"),
    ("error_kind.network", "网络错误"),
    ("error_kind.provider", "服务商错误"),
    ("error_kind.quota", "额度不足"),
    ("error_kind.rate_limited", "请求太频繁"),
    ("error_kind.server", "服务出错"),
    ("error_kind.storage", "存储错误"),
    ("error_kind.unknown", "未知错误"),
    ("header.context", "上下文 {percent}%"),
    ("inspector.empty", "还没有改动"),
    ("inspector.tab.changes", "改动"),
    ("inspector.tab.output", "工具输出"),
    ("inspector.tab.queue", "队列"),
    ("inspector.title", "Inspector"),
    ("menu.app.about", "关于 Forge"),
    ("menu.app.check_updates", "检查更新…"),
    ("menu.app.hide", "隐藏 Forge"),
    ("menu.app.hide_others", "隐藏其他"),
    ("menu.app.quit", "退出 Forge"),
    ("menu.app.settings", "设置…"),
    ("menu.app.show_all", "全部显示"),
    ("menu.edit", "编辑"),
    ("menu.edit.copy", "拷贝"),
    ("menu.edit.cut", "剪切"),
    ("menu.edit.paste", "粘贴"),
    ("menu.edit.redo", "重做"),
    ("menu.edit.select_all", "全选"),
    ("menu.edit.undo", "撤销"),
    ("menu.file", "文件"),
    ("menu.file.close_window", "关闭窗口"),
    ("menu.file.new_session", "新会话"),
    ("menu.session", "会话"),
    ("menu.session.focus_composer", "聚焦输入框"),
    ("menu.session.next", "下一个会话"),
    ("menu.session.prev", "上一个会话"),
    ("menu.session.retry", "重试上一次运行"),
    ("menu.session.stop", "停止运行"),
    ("menu.view", "显示"),
    ("menu.view.command_palette", "命令面板…"),
    ("menu.view.toggle_inspector", "显示/隐藏 Inspector"),
    ("menu.view.toggle_sidebar", "显示/隐藏侧栏"),
    ("menu.window", "窗口"),
    ("menu.window.minimize", "最小化"),
    ("mode.ask", "询问"),
    ("mode.ask.desc", "修改文件和运行命令前先问你（推荐）"),
    ("mode.read_only", "只读"),
    (
        "mode.read_only.desc",
        "只能读取和搜索，不会修改文件或运行命令",
    ),
    ("mode.unrestricted", "不受限"),
    (
        "mode.unrestricted.confirm.body",
        "Forge 将直接修改文件和运行命令，不再逐个询问。只在你信任当前任务时使用。",
    ),
    ("mode.unrestricted.confirm.cancel", "取消"),
    ("mode.unrestricted.confirm.ok", "切换"),
    ("mode.unrestricted.confirm.title", "切换到“不受限”？"),
    ("mode.unrestricted.desc", "直接执行，不再询问"),
    ("notice.compacted", "已压缩较早的上下文"),
    ("notice.permission", "已切换权限模式：{mode}"),
    ("onboarding.back", "上一步"),
    ("onboarding.done", "完成"),
    ("onboarding.key.later", "稍后再说"),
    ("onboarding.key.title", "添加 API Key"),
    ("onboarding.key.verify", "验证并继续"),
    ("onboarding.next", "下一步"),
    ("onboarding.notify", "后台需要批准时通知我"),
    ("onboarding.permissions.title", "权限与通知"),
    ("onboarding.start", "开始"),
    (
        "onboarding.welcome.body",
        "Forge 在你的电脑上运行，按你的允许读写项目文件和运行命令。这是全新的 app，不会读取旧版 Forge 的会话和设置。",
    ),
    ("onboarding.welcome.title", "欢迎使用 Forge"),
    (
        "onboarding.workspace.no_access",
        "没有读写权限，请换一个文件夹",
    ),
    (
        "onboarding.workspace.not_git",
        "不是 Git 仓库，检查点仍然可用，但 Git 工具不可用",
    ),
    ("onboarding.workspace.ok", "可以读写"),
    ("onboarding.workspace.pick", "选择项目文件夹…"),
    ("onboarding.workspace.title", "选择工作区"),
    ("palette.group.actions", "操作"),
    ("palette.group.sessions", "会话"),
    ("palette.new_session", "新会话"),
    ("palette.open_settings", "打开设置"),
    ("palette.placeholder", "搜索会话或操作…"),
    ("palette.stop", "停止"),
    ("palette.toggle_inspector", "显示/隐藏 Inspector"),
    ("palette.toggle_sidebar", "显示/隐藏侧栏"),
    ("queue.pause.cancelled", "你停止了运行"),
    ("queue.pause.failed", "上一条失败"),
    ("queue.pause.interrupted", "上次异常退出"),
    ("queue.pause.uncertain", "有状态未知的消息"),
    ("queue.pause.user", "你暂停了队列"),
    ("quit.confirm.body", "退出会停止它们。"),
    ("quit.confirm.cancel", "取消"),
    ("quit.confirm.ok", "停止并退出"),
    ("quit.confirm.title", "有 {n} 个运行进行中"),
    ("quit.stopping", "正在停止运行…"),
    ("reject.already_finished", "运行已经结束"),
    ("reject.approval_closed", "该请求已关闭"),
    ("reject.conflict", "内容和已提交的不一致"),
    ("reject.invalid_state", "当前状态不允许此操作"),
    ("reject.not_found", "找不到对象"),
    ("reject.not_implemented", "此功能还没有实现"),
    ("reject.run_finished", "运行已经结束"),
    ("reject.storage", "数据写入失败"),
    ("reject.transport", "无法连接后台"),
    ("reject.unavailable", "暂时不可用"),
    (
        "retry.marker",
        "连接中断（{reason}），{secs} 秒后第 {n} 次重试（共 5 次）",
    ),
    ("retry.now", "立即重试"),
    ("retry.stop", "停止"),
    ("row.cancelled", "已停止"),
    ("row.cancelling", "正在停止…"),
    ("row.failed", "失败 · {error_kind}"),
    ("row.failed_paused", "失败 · {error_kind} · 队列已暂停"),
    ("row.interrupted", "异常退出时中断"),
    ("row.running", "运行中 · {elapsed}"),
    ("row.uncertain", "{n} 条消息状态不确定"),
    ("row.waiting_approval", "等待批准 · {action}"),
    ("run.cancelled", "已停止"),
    ("run.failed", "失败"),
    ("run.interrupted", "异常退出时中断"),
    ("run.running", "运行中 · {elapsed}"),
    ("run.succeeded", "完成"),
    ("session.default_title", "新会话"),
    ("settings.key.base_url_hint", "留空使用官方地址"),
    (
        "settings.key.note",
        "Key 只保存在本机钥匙串里，不写入数据库或日志。",
    ),
    ("settings.key.status.failed", "✕ 验证失败：{reason}"),
    ("settings.key.status.saved", "已保存，未验证"),
    ("settings.key.status.testing", "正在验证…"),
    ("settings.key.status.unconfigured", "未配置"),
    ("settings.key.status.verified", "✓ 已验证"),
    (
        "settings.keychain_denied",
        "无法访问钥匙串，请在弹出的系统对话框中允许",
    ),
    ("settings.page.general", "通用"),
    ("settings.page.keys", "模型与 Key"),
    ("settings.page.permissions", "权限"),
    ("settings.page.usage", "用量"),
    ("settings.usage.empty", "还没有用量记录"),
    ("sidebar.archived", "归档（{n}）"),
    ("sidebar.bucket.older", "更早"),
    ("sidebar.bucket.today", "今天"),
    ("sidebar.bucket.week", "过去 7 天"),
    ("sidebar.bucket.yesterday", "昨天"),
    ("sidebar.empty", "还没有会话"),
    ("sidebar.group.attention", "需要处理"),
    ("sidebar.group.recent", "最近"),
    ("sidebar.group.running", "进行中"),
    ("sidebar.new_session", "新会话"),
    ("storage.action.logs", "打开日志目录"),
    ("storage.action.quit", "退出"),
    ("storage.action.reveal", "在 Finder 中显示"),
    ("storage.title", "无法打开 Forge 的数据"),
    ("sync.resyncing", "正在同步…"),
    ("time.min_sec", "{m} 分 {s} 秒"),
    ("time.sec", "{s} 秒"),
    ("timeline.from_queue", "来自队列"),
    ("timeline.jump_latest", "回到最新 ↓"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_and_unique() {
        for pair in ZH_CN.windows(2) {
            assert!(
                pair[0].0 < pair[1].0,
                "unsorted or duplicate key near {:?}",
                pair[1].0
            );
        }
    }

    #[test]
    fn lookup_and_format() {
        assert_eq!(t("composer.send"), "发送");
        assert_eq!(t("no.such.key"), "no.such.key");
        assert_eq!(tf("row.running", &[("elapsed", "1:23")]), "运行中 · 1:23");
        assert_eq!(tf("quit.confirm.title", &[("n", "2")]), "有 2 个运行进行中");
    }

    #[test]
    fn elapsed_format() {
        assert_eq!(format_elapsed(0), "0:00");
        assert_eq!(format_elapsed(83), "1:23");
        assert_eq!(format_elapsed(3725), "1:02:05");
    }
}
