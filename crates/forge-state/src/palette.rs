//! Command palette ⌘K (§6.15). Unavailable actions are hidden.

use crate::app::{AppState, CommandToken};
use crate::i18n::t;
use forge_protocol::SessionId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteAction {
    NewSession,
    StopRun,
    OpenSettings,
    ToggleSidebar,
    ToggleInspector,
    SwitchSession(SessionId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteItem {
    pub action: PaletteAction,
    pub label: String,
    /// Shortcut shown with `Kbd` (macOS glyphs).
    pub shortcut: Option<&'static str>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaletteVm {
    pub open: bool,
    pub query: String,
    pub selected: usize,
}

/// What the host must do after `palette_execute` (things outside forge-state).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteEffect {
    None,
    Sent(CommandToken),
    OpenSettings,
    ToggleInspector,
    FocusComposer,
}

impl AppState {
    pub fn open_palette(&mut self) {
        self.palette = PaletteVm {
            open: true,
            ..PaletteVm::default()
        };
    }

    /// Esc. The host returns focus to where it was (UI-CMD-01).
    pub fn close_palette(&mut self) {
        self.palette.open = false;
    }

    pub fn set_palette_query(&mut self, query: impl Into<String>) {
        self.palette.query = query.into();
        self.palette.selected = 0;
    }

    pub fn palette_move(&mut self, down: bool) {
        let len = self.palette_items().len();
        if len == 0 {
            self.palette.selected = 0;
        } else if down {
            self.palette.selected = (self.palette.selected + 1).min(len - 1);
        } else {
            self.palette.selected = self.palette.selected.saturating_sub(1);
        }
    }

    /// Filtered items: sessions first, then actions; case-insensitive match.
    pub fn palette_items(&self) -> Vec<PaletteItem> {
        let mut items: Vec<PaletteItem> = self
            .navigation_order()
            .into_iter()
            .filter_map(|id| self.sessions.get(&id))
            .map(|vm| PaletteItem {
                action: PaletteAction::SwitchSession(vm.session.id.clone()),
                label: vm.session.title.clone(),
                shortcut: None,
            })
            .collect();
        let can_create = self.active_workspace.is_some() && self.settings.settings.is_some();
        let running = self.selected().is_some_and(|vm| vm.is_busy());
        let actions = [
            (
                can_create,
                PaletteAction::NewSession,
                "palette.new_session",
                Some("⌘N"),
            ),
            (running, PaletteAction::StopRun, "palette.stop", Some("⌘.")),
            (
                true,
                PaletteAction::OpenSettings,
                "palette.open_settings",
                Some("⌘,"),
            ),
            (
                true,
                PaletteAction::ToggleSidebar,
                "palette.toggle_sidebar",
                Some("⌃⌘S"),
            ),
            (
                true,
                PaletteAction::ToggleInspector,
                "palette.toggle_inspector",
                Some("⌥⌘I"),
            ),
        ];
        items.extend(
            actions
                .into_iter()
                .filter(|(available, ..)| *available)
                .map(|(_, action, key, shortcut)| PaletteItem {
                    action,
                    label: t(key).into(),
                    shortcut,
                }),
        );
        let query = self.palette.query.trim().to_lowercase();
        if !query.is_empty() {
            items.retain(|item| item.label.to_lowercase().contains(&query));
        }
        items
    }

    /// ↩ on the selected item. Closes the palette.
    pub fn palette_execute(&mut self) -> PaletteEffect {
        let Some(item) = self.palette_items().into_iter().nth(self.palette.selected) else {
            return PaletteEffect::None;
        };
        self.palette.open = false;
        match item.action {
            PaletteAction::NewSession => self
                .create_session()
                .map_or(PaletteEffect::None, PaletteEffect::Sent),
            PaletteAction::StopRun => self
                .stop_run()
                .map_or(PaletteEffect::None, PaletteEffect::Sent),
            PaletteAction::OpenSettings => PaletteEffect::OpenSettings,
            PaletteAction::ToggleSidebar => {
                self.toggle_sidebar();
                PaletteEffect::None
            }
            PaletteAction::ToggleInspector => PaletteEffect::ToggleInspector,
            PaletteAction::SwitchSession(id) => {
                self.select_session(&id);
                PaletteEffect::FocusComposer
            }
        }
    }
}
