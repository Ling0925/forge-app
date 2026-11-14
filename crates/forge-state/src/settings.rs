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
    /// `None` until the host loads settings (no snapshot command yet).
    pub settings: Option<Settings>,
    pub key_status: BTreeMap<ProviderId, KeyStatus>,
    /// Inline error under the last changed item; value already reverted.
    pub error: Option<String>,
    pub confirm: Option<ConfirmDialog>,
}

impl SettingsVm {
    pub fn has_any_key(&self) -> bool {
        let configured = self
            .settings
            .as_ref()
            .is_some_and(|s| s.providers.iter().any(|p| p.keychain_reference.is_some()));
        configured
            || self
                .key_status
                .values()
                .any(|s| matches!(s, KeyStatus::Verified | KeyStatus::SavedUnverified))
    }
}

impl AppState {
    /// Host provides settings (from boot snapshot or after a save).
    pub fn load_settings(&mut self, settings: Settings) {
        for provider in &settings.providers {
            let status = if provider.keychain_reference.is_some() {
                KeyStatus::SavedUnverified
            } else {
                KeyStatus::Unconfigured
            };
            self.settings
                .key_status
                .entry(provider.id.clone())
                .or_insert(status);
        }
        self.settings.settings = Some(settings);
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
