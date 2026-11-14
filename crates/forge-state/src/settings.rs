//! Settings window and provider-key state (§6.13) plus shared confirm dialogs.

use crate::app::{AppState, CommandToken, Pending};
use crate::i18n::{t, tf};
use forge_protocol::{
    Command, ModelSelection, PermissionMode, ProviderId, ProviderKeyErrorKind, RejectCode,
    SessionId, Settings,
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyStatus {
    Unconfigured,
    SavedUnverified,
    Testing,
    Verified,
    Failed {
        kind: Option<ProviderKeyErrorKind>,
        message: Option<String>,
    },
}

impl KeyStatus {
    /// Localised status text for the provider row (§10 `settings.key.status`).
    pub fn label(&self) -> String {
        match self {
            Self::Unconfigured => t("settings.key.status.unconfigured").into(),
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
    /// `Snapshot.providers[].key_configured` / `ProviderKeyConfigured` — the
    /// only source for "is there a key" (drives the no-key banner and
    /// placeholder, §6.3 / §6.9).
    pub key_configured: BTreeMap<ProviderId, bool>,
    pub key_status: BTreeMap<ProviderId, KeyStatus>,
    /// The settings surface is open ("打开设置" on error cards, the no-key
    /// notice, ⌘,).
    pub open: bool,
    /// Inline error under the last changed item; value already reverted.
    pub error: Option<String>,
    pub confirm: Option<ConfirmDialog>,
}

impl SettingsVm {
    pub fn has_any_key(&self) -> bool {
        self.key_configured.values().any(|configured| *configured)
    }

    /// Key configured for the provider of the default model.
    pub fn has_key_for(&self, provider: &ProviderId) -> bool {
        self.key_configured.get(provider).copied().unwrap_or(false)
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
                .or_insert(KeyStatus::Unconfigured);
        }
        self.ui.apply_map(&settings.ui_prefs);
        self.settings.settings = Some(settings);
    }

    /// `ProviderKeyConfigured` (and each `Snapshot.providers` row).
    pub(crate) fn on_key_configured(&mut self, provider: ProviderId, configured: bool) {
        let status = self
            .settings
            .key_status
            .entry(provider.clone())
            .or_insert(KeyStatus::Unconfigured);
        let was = self.settings.key_configured.insert(provider, configured);
        *status = match (configured, was, &*status) {
            (false, _, _) => KeyStatus::Unconfigured,
            // Same key still there (snapshot after resync): keep the result
            // of an earlier test.
            (true, Some(true), KeyStatus::Verified | KeyStatus::Testing) => status.clone(),
            (true, _, _) => KeyStatus::SavedUnverified,
        };
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
            KeyStatus::Failed {
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
            .insert(provider.clone(), KeyStatus::Testing);
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
            KeyStatus::Verified
        } else {
            KeyStatus::Failed {
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
            KeyStatus::Failed {
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
