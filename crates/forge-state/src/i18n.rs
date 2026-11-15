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
    ("a11y.divider", "运行{outcome}"),
    ("a11y.inspector.tab", "{label}，{keys}"),
    ("a11y.reasoning", "思考过程"),
    ("a11y.reasoning.toggle", "{name}，{label}"),
    ("a11y.settings.close", "关闭设置"),
    ("a11y.settings.row", "{name}，{status}"),
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
    ("announce.keychain.recovered", "已重新读取钥匙串"),
    ("announce.keychain.still_unavailable", "仍无法读取钥匙串"),
    ("announce.queue_paused", "队列已暂停：{reason}"),
    ("announce.retry_ready", "现在可以重试"),
    ("announce.retrying", "连接中断，正在第 {n} 次重试"),
    ("announce.run.cancelled", "已停止"),
    ("announce.run.failed", "运行失败：{title}"),
    ("announce.run.started", "开始运行"),
    ("announce.run.succeeded", "运行完成，用时 {duration}"),
    ("announce.run.succeeded_unknown", "运行完成"),
    ("app.title", "Forge"),
    ("assistant.reasoning", "思考了 {secs} 秒"),
    ("assistant.stopped_suffix", "（已停止）"),
    ("assistant.thinking", "思考中… {elapsed}"),
    ("card.more", "更多操作"),
    ("card.status.failed", "失败"),
    ("composer.attach", "添加附件（即将支持）"),
    ("composer.hint.running", "↩ 加入队列 · ⌥↩ 插话"),
    ("composer.no_key", "先在设置里添加 API Key"),
    ("composer.not_sent", "没有发送：{reason}"),
    ("composer.open_settings", "打开设置"),
    ("composer.placeholder.idle", "描述你要做的事…"),
    ("composer.placeholder.running", "运行中"),
    ("composer.send", "发送"),
    ("composer.send_key", "↩ 发送 · ⇧↩ 换行"),
    ("composer.stop", "停止"),
    ("composer.stop_key", "停止 ⌘."),
    ("composer.stopping", "正在停止…"),
    ("divider.cost", "约 ${cost}"),
    ("divider.elapsed", "用时 {elapsed}"),
    ("divider.tokens", "输入 {input} / 输出 {output} tokens"),
    ("divider.usage_pending", "用量统计中"),
    ("empty.example.1", "解释这个项目的结构"),
    ("empty.example.2", "找出并修复一个失败的测试"),
    ("empty.example.3", "给最近的改动写说明"),
    ("empty.no_key", "还没有添加 API Key"),
    ("empty.title", "今天想让 Forge 做什么？"),
    ("error.action.details", "查看日志"),
    ("error.action.new", "新会话"),
    ("error.action.retry", "重试"),
    ("error.action.retry_wait", "重试（{remaining}）"),
    ("error.action.settings", "打开设置"),
    ("error.action.view_changes", "查看改动"),
    ("error.action.with_shortcut", "{label} {shortcut}"),
    ("error.auth.body", "请在设置里检查 {provider} 的 Key。"),
    ("error.auth.title", "API Key 无效或没有权限"),
    ("error.context.body", "已无法再压缩，建议新开会话继续。"),
    ("error.context.title", "对话太长"),
    ("error.copy_details", "复制错误信息"),
    ("error.detail.code", "错误码 {code}"),
    ("error.detail.header_redacted", "[已移除请求头]"),
    ("error.details.hide", "收起日志"),
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
    ("error.retry_after_ready", "现在可以重试了。"),
    ("error.server.body", "服务暂时不可用，已重试 5 次。"),
    ("error.server.title", "模型服务出错"),
    (
        "error.storage.body",
        "运行结果没能写入本地数据，详情里有原因。",
    ),
    ("error.storage.title", "保存失败"),
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
    ("inspector.empty.changes", "本会话还没有改动文件"),
    ("inspector.empty.output", "选中一个工具步骤查看完整输出"),
    ("inspector.empty.queue", "没有排队的消息"),
    ("inspector.queue.count", "{n} 条排队消息"),
    ("inspector.tab.changes", "改动"),
    ("inspector.tab.output", "工具输出"),
    ("inspector.tab.queue", "队列"),
    ("inspector.title", "Inspector"),
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
    ("menu.view.toggle_inspector", "显示/隐藏 Inspector"),
    ("menu.view.toggle_sidebar", "显示/隐藏侧栏"),
    ("menu.window", "窗口"),
    ("menu.window.minimize", "最小化"),
    ("notice.compacted", "已压缩较早的上下文"),
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
    ("palette.new_session", "新会话"),
    ("palette.open_settings", "打开设置"),
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
    ("retry.stop", "停止"),
    ("row.cancelled", "已停止"),
    ("row.cancelling", "正在停止…"),
    ("row.failed", "失败 · {error_kind}"),
    ("row.failed_paused", "失败 · {error_kind} · 队列已暂停"),
    ("row.interrupted", "异常退出时中断"),
    ("row.running", "运行中 · {elapsed}"),
    ("row.running_unknown", "运行中"),
    ("row.uncertain", "{n} 条消息状态不确定"),
    ("row.waiting_approval", "等待批准 · {action}"),
    ("run.cancelled", "已停止"),
    ("run.failed", "失败"),
    ("run.interrupted", "异常退出时中断"),
    ("run.running", "运行中 · {elapsed}"),
    ("run.running_unknown", "运行中"),
    ("run.succeeded", "完成"),
    ("session.default_title", "新会话"),
    ("settings.about.version", "{version}"),
    ("settings.close", "关闭"),
    ("settings.group.about", "关于"),
    ("settings.group.appearance", "外观"),
    ("settings.group.services", "模型服务"),
    (
        "settings.key.note",
        "Key 只保存在本机钥匙串里，不写入数据库或日志。",
    ),
    ("settings.key.placeholder", "粘贴 API Key"),
    ("settings.key.remove", "移除"),
    ("settings.key.save", "保存"),
    ("settings.key.status.failed", "✕ 验证失败：{reason}"),
    ("settings.key.status.saved", "已保存，未验证"),
    ("settings.key.status.testing", "正在验证…"),
    ("settings.key.status.unavailable", "无法读取钥匙串"),
    ("settings.key.status.unconfigured", "未配置"),
    ("settings.key.status.verified", "✓ 已验证"),
    ("settings.key.unavailable_retry", "重试"),
    (
        "settings.key.unavailable_retry_a11y",
        "重试读取 {provider} 的钥匙串",
    ),
    (
        "settings.keychain_denied",
        "无法访问钥匙串，请在弹出的系统对话框中允许",
    ),
    ("settings.model.label", "默认模型"),
    ("settings.model.note", "新会话使用的模型"),
    ("settings.page", "设置"),
    ("settings.theme", "主题"),
    ("settings.theme.dark", "深色"),
    ("settings.theme.light", "浅色"),
    ("settings.theme.system", "跟随系统"),
    ("sidebar.archived", "归档"),
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
    ("storage.detail", "数据位置：{path}（错误码 {code}）"),
    ("storage.title", "无法打开 Forge 的数据"),
    ("time.days_ago", "{n} 天前"),
    ("time.hours_ago", "{n} 小时前"),
    ("time.just_now", "刚刚"),
    ("time.min_sec", "{m} 分 {s} 秒"),
    ("time.minutes_ago", "{n} 分钟前"),
    ("time.sec", "{s} 秒"),
    ("timeline.from_queue", "来自队列"),
    ("timeline.jump_latest", "回到最新 ↓"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::ffi::OsStr;
    use std::fs;
    use std::path::{Path, PathBuf};

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

    /// Key-shaped literals that are not i18n keys: `ui_prefs` setting names.
    const NON_KEY_LITERALS: &[&str] = &[
        "appearance.theme",
        "inspector.visible",
        "inspector.width",
        "sidebar.visible",
        "sidebar.width",
    ];

    #[test]
    fn production_keys_all_exist() {
        let (referenced, templates) = referenced_literals();
        let keys: BTreeSet<&str> = ZH_CN.iter().map(|(key, _)| *key).collect();
        for lit in referenced {
            assert!(
                keys.contains(lit.as_str()),
                "production code references {lit:?}, which is not in ZH_CN"
            );
        }
        for tpl in templates {
            assert!(
                ZH_CN.iter().any(|(key, _)| template_covers(&tpl, key)),
                "key template {tpl:?} matches no ZH_CN key"
            );
        }
    }

    #[test]
    fn no_dead_keys() {
        let (referenced, templates) = referenced_literals();
        for (key, _) in ZH_CN {
            let used =
                referenced.contains(*key) || templates.iter().any(|tpl| template_covers(tpl, key));
            assert!(used, "ZH_CN key {key:?} is never referenced");
        }
    }

    /// Every key-shaped string literal in the production sources of
    /// forge-state and forge-ui, split into plain references and `{x}` key
    /// templates (e.g. `"empty.example.{i}"`). `#[cfg(test)]` modules,
    /// comments, `ui_prefs` names, and the gpui-kit colour-name mapping table
    /// do not count as references.
    fn referenced_literals() -> (BTreeSet<String>, BTreeSet<String>) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut referenced = BTreeSet::new();
        let mut templates = BTreeSet::new();
        for dir in [root.join("src"), root.join("../forge-ui/src")] {
            let mut files = Vec::new();
            rs_files(&dir, &mut files);
            for file in files {
                if file.file_name() == Some(OsStr::new("i18n.rs")) {
                    continue;
                }
                let mut src = fs::read_to_string(&file).unwrap();
                if let Some(i) = src.find("#[cfg(test)]") {
                    src.truncate(i);
                }
                if let Some(i) = src.find("GPUI_KIT_MAPPING") {
                    let end = src[i..].find("];").map(|j| i + j + 2).unwrap_or(src.len());
                    src.replace_range(i..end, "");
                }
                for lit in string_literals(&src) {
                    if !is_key_literal(&lit) || NON_KEY_LITERALS.contains(&lit.as_str()) {
                        continue;
                    }
                    if lit.contains('{') {
                        templates.insert(lit);
                    } else {
                        referenced.insert(lit);
                    }
                }
            }
        }
        (referenced, templates)
    }

    fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                rs_files(&path, out);
            } else if path.extension() == Some(OsStr::new("rs")) {
                out.push(path);
            }
        }
    }

    /// `key` matches `template` when they have the same segments and every
    /// `{x}` segment in the template stands in for the real segment.
    fn template_covers(template: &str, key: &str) -> bool {
        let t: Vec<&str> = template.split('.').collect();
        let k: Vec<&str> = key.split('.').collect();
        t.len() == k.len()
            && t.iter()
                .zip(&k)
                .all(|(t, k)| *t == *k || (t.starts_with('{') && t.ends_with('}')))
    }

    /// Dotted lowercase segments: `composer.send`, `a11y.sidebar.row`,
    /// `empty.example.{i}` (a `{x}` segment marks a format template).
    fn is_key_literal(s: &str) -> bool {
        s.contains('.')
            && s.split('.').all(|seg| {
                if let Some(inner) = seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                    !inner.is_empty()
                        && inner
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                } else {
                    seg.chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
                        && seg
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                }
            })
    }

    /// Collects `"…"` literal contents, skipping `//` and `/* */` comments,
    /// `'…'` char literals (so e.g. `'"'` does not unbalance the scan) and
    /// `r"…"`/`r#"…"#`/`b"…"`/`br"…"` raw or byte strings.
    fn string_literals(src: &str) -> Vec<String> {
        fn at(cs: &[char], i: usize) -> Option<char> {
            cs.get(i).copied()
        }
        let cs: Vec<char> = src.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < cs.len() {
            match cs[i] {
                '/' if at(&cs, i + 1) == Some('/') => {
                    while i < cs.len() && cs[i] != '\n' {
                        i += 1;
                    }
                }
                '/' if at(&cs, i + 1) == Some('*') => {
                    i += 2;
                    while i + 1 < cs.len() && !(cs[i] == '*' && cs[i + 1] == '/') {
                        i += 1;
                    }
                    i = (i + 2).min(cs.len());
                }
                '\'' => {
                    // 'c' | '\\x' | '\u{..}' — a lifetime has no closing quote.
                    let mut close = 0;
                    if at(&cs, i + 1) == Some('\\') {
                        if at(&cs, i + 2) == Some('u') && at(&cs, i + 3) == Some('{') {
                            let mut j = i + 4;
                            while j < cs.len() && cs[j] != '}' {
                                j += 1;
                            }
                            if at(&cs, j + 1) == Some('\'') {
                                close = j + 1;
                            }
                        } else if at(&cs, i + 3) == Some('\'') {
                            close = i + 3;
                        }
                    } else if at(&cs, i + 2) == Some('\'') {
                        close = i + 2;
                    }
                    i = if close > 0 { close + 1 } else { i + 1 };
                }
                'r' | 'b' => {
                    let mut j = i;
                    if cs[j] == 'b' {
                        j += 1;
                        if cs.get(j) == Some(&'r') {
                            j += 1;
                        }
                    } else {
                        j += 1;
                    }
                    let mut hashes = 0;
                    while cs.get(j) == Some(&'#') {
                        j += 1;
                        hashes += 1;
                    }
                    if cs.get(j) != Some(&'"') {
                        i += 1;
                        continue;
                    }
                    // A raw/byte string ends at `"` + the same # run; escapes
                    // apply only to b"…" (hashes == 0 && starts with b).
                    let escaped = hashes == 0 && cs[i] == 'b';
                    let start = j + 1;
                    let mut k = start;
                    while k < cs.len() {
                        if escaped && cs[k] == '\\' {
                            k += 2;
                            continue;
                        }
                        if cs[k] == '"' && (0..hashes).all(|h| cs.get(k + 1 + h) == Some(&'#')) {
                            break;
                        }
                        k += 1;
                    }
                    out.push(cs[start..k.min(cs.len())].iter().collect());
                    i = (k + 1 + hashes).min(cs.len());
                }
                '"' => {
                    let mut buf = String::new();
                    let mut j = i + 1;
                    while j < cs.len() && cs[j] != '"' {
                        if cs[j] == '\\' {
                            j += 1;
                        }
                        buf.push(cs[j]);
                        j += 1;
                    }
                    out.push(buf);
                    i = j + 1;
                }
                _ => i += 1,
            }
        }
        out
    }
}
