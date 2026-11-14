//! Window / three-column layout (spec §4) and the UI preferences behind it.
//!
//! `UiPrefs` is the forge-state side of `Settings.ui_prefs` (a string map Core
//! stores verbatim): keys `sidebar.width`, `sidebar.visible`,
//! `inspector.width`, `inspector.visible`.

use std::collections::BTreeMap;

pub const PREF_SIDEBAR_WIDTH: &str = "sidebar.width";
pub const PREF_SIDEBAR_VISIBLE: &str = "sidebar.visible";
pub const PREF_INSPECTOR_WIDTH: &str = "inspector.width";
pub const PREF_INSPECTOR_VISIBLE: &str = "inspector.visible";

/// Main window minimum (§4).
pub const MIN_WINDOW_WIDTH: f32 = 960.0;
pub const MIN_WINDOW_HEIGHT: f32 = 600.0;
pub const DEFAULT_WINDOW_WIDTH: f32 = 1280.0;
pub const DEFAULT_WINDOW_HEIGHT: f32 = 800.0;

pub const SIDEBAR_DEFAULT: f32 = 240.0;
pub const SIDEBAR_MIN: f32 = 200.0;
pub const SIDEBAR_MAX: f32 = 360.0;
/// Dragging narrower than this collapses the sidebar (§4 "拖到 <160 自动收起").
pub const SIDEBAR_COLLAPSE_BELOW: f32 = 160.0;

pub const INSPECTOR_DEFAULT: f32 = 360.0;
pub const INSPECTOR_MIN: f32 = 280.0;
pub const INSPECTOR_MAX: f32 = 560.0;

pub const MAIN_MIN: f32 = 480.0;
/// Timeline / composer content column (§4 "时间线内容最大宽 760").
pub const CONTENT_MAX: f32 = 760.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiPrefs {
    pub sidebar_visible: bool,
    pub sidebar_width: f32,
    pub inspector_visible: bool,
    pub inspector_width: f32,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            sidebar_visible: true,
            sidebar_width: SIDEBAR_DEFAULT,
            // §4: Inspector is closed by default.
            inspector_visible: false,
            inspector_width: INSPECTOR_DEFAULT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PanelLayout {
    Hidden,
    Docked {
        width: f32,
    },
    /// Narrow window: floats over the main area.
    Overlay {
        width: f32,
    },
}

impl PanelLayout {
    pub fn docked_width(self) -> f32 {
        match self {
            Self::Docked { width } => width,
            _ => 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutVm {
    pub sidebar: PanelLayout,
    pub inspector: PanelLayout,
    pub main_width: f32,
}

/// Result of dragging the sidebar divider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SidebarResize {
    Resized(f32),
    Collapsed,
}

impl UiPrefs {
    /// Restores values from `Settings.ui_prefs`; missing or unparsable keys
    /// keep their current value, widths are clamped to the §4 ranges.
    pub fn apply_map(&mut self, map: &BTreeMap<String, String>) {
        let number = |key: &str| map.get(key).and_then(|v| v.parse::<f32>().ok());
        let flag = |key: &str| map.get(key).and_then(|v| v.parse::<bool>().ok());
        if let Some(w) = number(PREF_SIDEBAR_WIDTH).filter(|w| w.is_finite()) {
            self.sidebar_width = w.clamp(SIDEBAR_MIN, SIDEBAR_MAX);
        }
        if let Some(v) = flag(PREF_SIDEBAR_VISIBLE) {
            self.sidebar_visible = v;
        }
        if let Some(w) = number(PREF_INSPECTOR_WIDTH).filter(|w| w.is_finite()) {
            self.inspector_width = w.clamp(INSPECTOR_MIN, INSPECTOR_MAX);
        }
        if let Some(v) = flag(PREF_INSPECTOR_VISIBLE) {
            self.inspector_visible = v;
        }
    }

    /// Writes the layout keys, leaving other `ui_prefs` keys untouched.
    pub fn write_map(&self, map: &mut BTreeMap<String, String>) {
        map.insert(
            PREF_SIDEBAR_WIDTH.into(),
            format!("{}", self.sidebar_width.round()),
        );
        map.insert(
            PREF_SIDEBAR_VISIBLE.into(),
            self.sidebar_visible.to_string(),
        );
        map.insert(
            PREF_INSPECTOR_WIDTH.into(),
            format!("{}", self.inspector_width.round()),
        );
        map.insert(
            PREF_INSPECTOR_VISIBLE.into(),
            self.inspector_visible.to_string(),
        );
    }

    /// Divider drag: clamps to 200–360, collapses below 160.
    pub fn drag_sidebar(&mut self, width: f32) -> SidebarResize {
        if width < SIDEBAR_COLLAPSE_BELOW {
            self.sidebar_visible = false;
            return SidebarResize::Collapsed;
        }
        self.sidebar_visible = true;
        self.sidebar_width = width.clamp(SIDEBAR_MIN, SIDEBAR_MAX);
        SidebarResize::Resized(self.sidebar_width)
    }

    /// Divider double-click.
    pub fn reset_sidebar_width(&mut self) {
        self.sidebar_width = SIDEBAR_DEFAULT;
        self.sidebar_visible = true;
    }

    pub fn drag_inspector(&mut self, width: f32) {
        self.inspector_width = width.clamp(INSPECTOR_MIN, INSPECTOR_MAX);
    }

    pub fn reset_inspector_width(&mut self) {
        self.inspector_width = INSPECTOR_DEFAULT;
    }

    /// §4 narrow-window rules for a window `width` points wide.
    pub fn layout(&self, width: f32) -> LayoutVm {
        let sidebar_w = self.sidebar_width.clamp(SIDEBAR_MIN, SIDEBAR_MAX);
        let inspector_w = self.inspector_width.clamp(INSPECTOR_MIN, INSPECTOR_MAX);
        let sidebar = if !self.sidebar_visible {
            PanelLayout::Hidden
        } else if width < sidebar_w + MAIN_MIN {
            // "宽度 < 侧栏 + 480 时自动收起侧栏".
            PanelLayout::Hidden
        } else {
            PanelLayout::Docked { width: sidebar_w }
        };
        let inspector = if !self.inspector_visible {
            PanelLayout::Hidden
        } else if width < sidebar.docked_width() + MAIN_MIN + inspector_w {
            PanelLayout::Overlay { width: inspector_w }
        } else {
            PanelLayout::Docked { width: inspector_w }
        };
        LayoutVm {
            sidebar,
            inspector,
            main_width: (width - sidebar.docked_width() - inspector.docked_width()).max(0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_spec() {
        let prefs = UiPrefs::default();
        assert_eq!(prefs.sidebar_width, 240.0);
        assert_eq!(prefs.inspector_width, 360.0);
        assert!(!prefs.inspector_visible);
    }
}
