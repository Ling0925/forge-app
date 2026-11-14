//! First-launch wizard (§6.14): 欢迎 → 选择工作区 → 添加 API Key → 权限与通知.

use crate::app::{AppState, CommandToken};
use forge_protocol::{PermissionMode, ProviderId};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OnboardingStep {
    Welcome,
    Workspace,
    ApiKey,
    Permissions,
}

impl OnboardingStep {
    /// 1-based index for the `Stepper`.
    pub fn index(self) -> usize {
        self as usize + 1
    }
}

/// Folder check result, computed by the host (it owns the file system).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceCheck {
    ReadWrite,
    NotGit,
    NoAccess,
}

impl WorkspaceCheck {
    pub fn label_key(self) -> &'static str {
        match self {
            Self::ReadWrite => "onboarding.workspace.ok",
            Self::NotGit => "onboarding.workspace.not_git",
            Self::NoAccess => "onboarding.workspace.no_access",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingVm {
    pub step: OnboardingStep,
    pub workspace: Option<(PathBuf, WorkspaceCheck)>,
    pub workspace_submitted: Option<PathBuf>,
    /// Provider whose key is being verified (button spinner).
    pub validating: Option<ProviderId>,
    pub error: Option<String>,
    pub key_skipped: bool,
    pub permission_mode: PermissionMode,
    pub notify_on_approval: bool,
}

impl Default for OnboardingVm {
    fn default() -> Self {
        Self {
            step: OnboardingStep::Welcome,
            workspace: None,
            workspace_submitted: None,
            validating: None,
            error: None,
            key_skipped: false,
            permission_mode: PermissionMode::Ask,
            notify_on_approval: false,
        }
    }
}

impl AppState {
    pub fn start_onboarding(&mut self) {
        self.onboarding = Some(OnboardingVm::default());
    }

    pub fn onboarding_choose_workspace(&mut self, path: PathBuf, check: WorkspaceCheck) {
        if let Some(vm) = self.onboarding.as_mut() {
            vm.workspace = Some((path, check));
            vm.error = None;
        }
    }

    /// "下一步" enabled? (UI-OB-03: no-access folder disables it.)
    pub fn onboarding_can_advance(&self) -> bool {
        let Some(vm) = self.onboarding.as_ref() else {
            return false;
        };
        match vm.step {
            OnboardingStep::Welcome | OnboardingStep::Permissions => true,
            OnboardingStep::Workspace => vm
                .workspace
                .as_ref()
                .is_some_and(|(_, check)| *check != WorkspaceCheck::NoAccess),
            // Leaves the key step only via "验证并继续" or "稍后再说".
            OnboardingStep::ApiKey => false,
        }
    }

    /// "开始" / "下一步" / "完成".
    pub fn onboarding_next(&mut self) -> Vec<CommandToken> {
        if !self.onboarding_can_advance() {
            return Vec::new();
        }
        let vm = self.onboarding.as_mut().expect("checked above");
        match vm.step {
            OnboardingStep::Welcome => {
                vm.step = OnboardingStep::Workspace;
                Vec::new()
            }
            OnboardingStep::Workspace => {
                let path = vm.workspace.as_ref().expect("checked").0.clone();
                vm.step = OnboardingStep::ApiKey;
                if vm.workspace_submitted.as_ref() == Some(&path) {
                    return Vec::new();
                }
                vm.workspace_submitted = Some(path.clone());
                vec![self.add_workspace(path)]
            }
            OnboardingStep::ApiKey => Vec::new(),
            OnboardingStep::Permissions => self.onboarding_finish(),
        }
    }

    /// "上一步" / ⌘[.
    pub fn onboarding_back(&mut self) {
        if let Some(vm) = self.onboarding.as_mut() {
            vm.error = None;
            vm.validating = None;
            vm.step = match vm.step {
                OnboardingStep::Welcome | OnboardingStep::Workspace => OnboardingStep::Welcome,
                OnboardingStep::ApiKey => OnboardingStep::Workspace,
                OnboardingStep::Permissions => OnboardingStep::ApiKey,
            };
        }
    }

    /// "验证并继续" for one provider (the key itself is saved to Keychain by the
    /// host before this call; the protocol only carries the provider id).
    pub fn onboarding_verify_key(&mut self, provider: ProviderId) -> Option<CommandToken> {
        let vm = self.onboarding.as_mut()?;
        if vm.step != OnboardingStep::ApiKey || vm.validating.is_some() {
            return None;
        }
        vm.validating = Some(provider.clone());
        vm.error = None;
        Some(self.test_provider_key(provider))
    }

    pub(crate) fn onboarding_key_tested(&mut self, provider: &ProviderId, ok: bool) {
        let label = self
            .settings
            .key_status
            .get(provider)
            .map(|status| status.label());
        let Some(vm) = self.onboarding.as_mut() else {
            return;
        };
        if vm.validating.as_ref() != Some(provider) {
            return;
        }
        vm.validating = None;
        if ok {
            vm.step = OnboardingStep::Permissions;
            vm.key_skipped = false;
        } else {
            vm.error = label;
        }
    }

    /// "稍后再说": continue without a key; the composer shows the NoKey state.
    pub fn onboarding_skip_key(&mut self) {
        if let Some(vm) = self.onboarding.as_mut()
            && vm.step == OnboardingStep::ApiKey
        {
            vm.key_skipped = true;
            vm.validating = None;
            vm.error = None;
            vm.step = OnboardingStep::Permissions;
        }
    }

    pub fn onboarding_set_permission_mode(&mut self, mode: PermissionMode) {
        if let Some(vm) = self.onboarding.as_mut() {
            vm.permission_mode = mode;
        }
    }

    pub fn onboarding_set_notify(&mut self, on: bool) {
        if let Some(vm) = self.onboarding.as_mut() {
            vm.notify_on_approval = on;
        }
    }

    /// Saves the permission choice and opens a new empty session.
    fn onboarding_finish(&mut self) -> Vec<CommandToken> {
        let Some(vm) = self.onboarding.take() else {
            return Vec::new();
        };
        let mut tokens = Vec::new();
        if let Some(token) = self.request_default_permission_mode(vm.permission_mode.clone()) {
            tokens.push(token);
        }
        if vm.permission_mode == PermissionMode::Unrestricted {
            // The wizard step itself is the explicit choice; no second dialog.
            if let Some(token) = self.confirm_dialog_accept() {
                tokens.push(token);
            }
        }
        if let Some(token) = self.create_session() {
            tokens.push(token);
        }
        tokens
    }
}
