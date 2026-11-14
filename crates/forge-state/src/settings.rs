//! Settings window and provider-key state (§6.13) plus shared confirm dialogs.

use crate::app::{AnnouncementKind, AnnouncementPriority, AppState, CommandToken, Pending};
use crate::i18n::{t, tf};
use forge_protocol::{
    Command, KeyStatus, ModelSelection, PermissionMode, ProviderId, ProviderKeyErrorKind,
    RejectCode, SessionId, Settings,
};
use std::collections::BTreeMap;

/// UI-side state of one provider key row: Core's `KeyStatus` plus the
/// local save / test lifecycle. Named apart from `forge_protocol::KeyStatus`
/// so both can be glob-imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalKeyState {
    Unconfigured,
    /// Core `key_status: unavailable` — the keychain could not be read. Never
    /// shown as the "未配置" (missing) UI.
    Unavailable,
    SavedUnverified,
    Testing,
    Verified,
    Failed {
        kind: Option<ProviderKeyErrorKind>,
        message: Option<String>,
    },
}

impl LocalKeyState {
    /// Localised status text for the provider row (§10 `settings.key.status`).
    pub fn label(&self) -> String {
        match self {
            Self::Unconfigured => t("settings.key.status.unconfigured").into(),
            Self::Unavailable => t("settings.key.status.unavailable").into(),
            Self::SavedUnverified => t("settings.key.status.saved").into(),
            Self::Testing => t("settings.key.status.testing").into(),
            Self::Verified => t("settings.key.status.verified").into(),
            Self::Failed { kind, message } => {
                let reason = match kind {
                    Some(kind) => t(key_error_label(kind)).to_owned(),
                    None => message.clone().unwrap_or_default(),
                };
                tf("settings.key.status.failed", &[("reason", &reason)])
            }
        }
    }
}

fn key_error_label(kind: &ProviderKeyErrorKind) -> &'static str {
    match kind {
        ProviderKeyErrorKind::Network => "error_kind.network",
        ProviderKeyErrorKind::RateLimited => "error_kind.rate_limited",
        ProviderKeyErrorKind::Auth => "error_kind.auth",
        ProviderKeyErrorKind::Quota => "error_kind.quota",
        ProviderKeyErrorKind::Server => "error_kind.server",
        ProviderKeyErrorKind::Configuration => "error_kind.unknown",
        ProviderKeyErrorKind::Provider => "error_kind.provider",
    }
}

/// Confirm dialogs that gate a command (§6.2 / §6.13 "不受限").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmDialog {
    UnrestrictedDefault,
    UnrestrictedSession(SessionId),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SettingsVm {
    /// `None` until the first `Snapshot` / `SettingsUpdated`.
    pub settings: Option<Settings>,
    /// `Snapshot.providers[].key_status` / `ProviderKeyConfigured.key_status`
    /// — the only source for "is there a key" (drives the no-key banner and
    /// placeholder, §6.3 / §6.9).
    pub core_key_status: BTreeMap<ProviderId, KeyStatus>,
    pub key_status: BTreeMap<ProviderId, LocalKeyState>,
    /// "重试" on an unavailable keychain row sent `GetSnapshot`; the next
    /// Snapshot (or its rejection) announces the result once, politely.
    pub key_retry_pending: bool,
    /// The settings surface is open ("打开设置" on error cards, the no-key
    /// notice, ⌘,).
    pub open: bool,
    /// Inline error under the last changed item; value already reverted.
    pub error: Option<String>,
    pub confirm: Option<ConfirmDialog>,
}

impl SettingsVm {
    pub fn has_any_key(&self) -> bool {
        self.core_key_status
            .values()
            .any(|status| *status == KeyStatus::Configured)
    }

    /// Key configured for the provider of the default model.
    pub fn has_key_for(&self, provider: &ProviderId) -> bool {
        self.core_key_status.get(provider) == Some(&KeyStatus::Configured)
    }

    /// Core could not read this provider's key from the keychain.
    pub fn key_unavailable(&self, provider: &ProviderId) -> bool {
        self.core_key_status.get(provider) == Some(&KeyStatus::Unavailable)
    }

    pub fn any_key_unavailable(&self) -> bool {
        self.core_key_status
            .values()
            .any(|status| *status == KeyStatus::Unavailable)
    }

    /// The "未配置 Key" UI (banner, composer placeholder): no configured key
    /// and no provider whose keychain is merely unreadable. `unavailable`
    /// never counts as missing.
    pub fn key_missing(&self) -> bool {
        !self.has_any_key() && !self.any_key_unavailable()
    }
}

impl AppState {
    /// Settings from `Snapshot` / `SettingsUpdated`: also restores the layout
    /// stored in `Settings.ui_prefs`.
    pub fn load_settings(&mut self, settings: Settings) {
        for provider in &settings.providers {
            self.settings
                .key_status
                .entry(provider.id.clone())
                .or_insert(LocalKeyState::Unconfigured);
        }
        self.ui.apply_map(&settings.ui_prefs);
        self.settings.settings = Some(settings);
    }

    /// `ProviderKeyConfigured` (and each `Snapshot.providers` row).
    pub(crate) fn on_key_status(&mut self, provider: ProviderId, key_status: KeyStatus) {
        let local = self
            .settings
            .key_status
            .entry(provider.clone())
            .or_insert(LocalKeyState::Unconfigured);
        let was = self
            .settings
            .core_key_status
            .insert(provider, key_status.clone());
        *local = match (&key_status, was, &*local) {
            (KeyStatus::Missing, _, _) => LocalKeyState::Unconfigured,
            (KeyStatus::Unavailable, _, _) => LocalKeyState::Unavailable,
            // Same key still there (snapshot after resync): keep the result
            // of an earlier test.
            (
                KeyStatus::Configured,
                Some(KeyStatus::Configured),
                LocalKeyState::Verified | LocalKeyState::Testing,
            ) => local.clone(),
            (KeyStatus::Configured, _, _) => LocalKeyState::SavedUnverified,
        };
    }

    /// "重试" next to "无法读取钥匙串": re-sends `GetSnapshot` (Core re-reads
    /// the keychain for every provider). At most one snapshot is in flight;
    /// a second press while waiting sends nothing.
    pub fn retry_key_status(&mut self) -> Option<CommandToken> {
        self.settings.key_retry_pending = true;
        self.request_snapshot()
    }

    /// After the Snapshot that answers a keychain retry: one polite
    /// announcement with the result.
    pub(crate) fn announce_key_retry_result(&mut self) {
        if !std::mem::take(&mut self.settings.key_retry_pending) {
            return;
        }
        let key = if self.settings.any_key_unavailable() {
            "announce.keychain.still_unavailable"
        } else {
            "announce.keychain.recovered"
        };
        self.announce(
            AnnouncementKind::Other,
            AnnouncementPriority::Medium,
            t(key).into(),
        );
    }

    /// The retry's `GetSnapshot` was rejected: still unreadable.
    pub(crate) fn on_key_retry_rejected(&mut self) {
        if std::mem::take(&mut self.settings.key_retry_pending) {
            self.announce(
                AnnouncementKind::Other,
                AnnouncementPriority::Medium,
                t("announce.keychain.still_unavailable").into(),
            );
        }
    }

    /// "添加 / 更换"(`Some`) or "移除"(`None`): `SetProviderKey`. The key goes
    /// straight into the command and is not kept in UI state.
    pub fn set_provider_key(&mut self, provider: ProviderId, key: Option<String>) -> CommandToken {
        let key = key.map(|k| k.trim().to_owned()).filter(|k| !k.is_empty());
        self.settings.error = None;
        self.push(
            Pending::SetKey {
                provider: provider.clone(),
            },
            Command::SetProviderKey { provider, key },
        )
    }

    pub(crate) fn on_key_set_rejected(
        &mut self,
        provider: ProviderId,
        code: RejectCode,
        message: String,
    ) {
        // Keychain refused (Core answers `unavailable`).
        let text = if code == RejectCode::Unavailable {
            t("settings.keychain_denied").to_owned()
        } else {
            crate::app::not_sent(&code, &message)
        };
        self.settings.key_status.insert(
            provider,
            LocalKeyState::Failed {
                kind: None,
                message: Some(text.clone()),
            },
        );
        self.settings.error = Some(text);
    }

    pub fn open_settings(&mut self) {
        self.settings.open = true;
    }

    pub fn close_settings(&mut self) {
        self.settings.open = false;
    }

    /// Writes the current layout into `Settings.ui_prefs` (one
    /// `UpdateSettings`; nothing when unchanged). Call at drag end / toggle.
    pub fn persist_ui_prefs(&mut self) -> Option<CommandToken> {
        let prefs = self.ui;
        self.update_settings(|s| prefs.write_map(&mut s.ui_prefs))
    }

    /// "验证" button (G2 `TestProviderKey`).
    pub fn test_provider_key(&mut self, provider: ProviderId) -> CommandToken {
        self.settings
            .key_status
            .insert(provider.clone(), LocalKeyState::Testing);
        self.push(
            Pending::TestKey {
                provider: provider.clone(),
            },
            Command::TestProviderKey { provider },
        )
    }

    pub(crate) fn on_key_tested(
        &mut self,
        provider: ProviderId,
        ok: bool,
        error_kind: Option<ProviderKeyErrorKind>,
    ) {
        let status = if ok {
            LocalKeyState::Verified
        } else {
            LocalKeyState::Failed {
                kind: error_kind,
                message: None,
            }
        };
        self.settings.key_status.insert(provider.clone(), status);
        self.onboarding_key_tested(&provider, ok);
    }

    pub(crate) fn on_key_test_rejected(
        &mut self,
        provider: ProviderId,
        code: RejectCode,
        message: String,
    ) {
        self.settings.key_status.insert(
            provider.clone(),
            LocalKeyState::Failed {
                kind: None,
                message: Some(crate::app::not_sent(&code, &message)),
            },
        );
        self.onboarding_key_tested(&provider, false);
    }

    fn update_settings(&mut self, change: impl FnOnce(&mut Settings)) -> Option<CommandToken> {
        let current = self.settings.settings.as_mut()?;
        let previous = current.clone();
        change(current);
        if *current == previous {
            return None;
        }
        let settings = current.clone();
        self.settings.error = None;
        Some(self.push(
            Pending::UpdateSettings { previous },
            Command::UpdateSettings { settings },
        ))
    }

    pub fn set_default_model(&mut self, model: ModelSelection) -> Option<CommandToken> {
        self.update_settings(|s| s.model = model)
    }

    /// Default permission mode; `Unrestricted` asks first (UI-ST-04).
    pub fn request_default_permission_mode(
        &mut self,
        mode: PermissionMode,
    ) -> Option<CommandToken> {
        if mode == PermissionMode::Unrestricted {
            let current = self.settings.settings.as_ref()?.permission_mode.clone();
            if current != PermissionMode::Unrestricted {
                self.settings.confirm = Some(ConfirmDialog::UnrestrictedDefault);
            }
            return None;
        }
        self.update_settings(|s| s.permission_mode = mode)
    }

    pub fn confirm_dialog_accept(&mut self) -> Option<CommandToken> {
        match self.settings.confirm.take()? {
            ConfirmDialog::UnrestrictedDefault => {
                self.update_settings(|s| s.permission_mode = PermissionMode::Unrestricted)
            }
            ConfirmDialog::UnrestrictedSession(session) => {
                self.send_session_permission_mode(&session, PermissionMode::Unrestricted)
            }
        }
    }

    /// Cancel keeps the previous value (UI-TB-02, UI-ST-04).
    pub fn confirm_dialog_cancel(&mut self) {
        self.settings.confirm = None;
    }
}
