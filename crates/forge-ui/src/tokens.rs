//! Forge design tokens (FORGE_APP_UI_SPEC.md §3.1) and the WCAG contrast check
//! behind the §3.1 table. Pure Rust so it is unit-tested on Linux; the gpui-kit
//! theme JSON in `themes/forge.json` is checked against these values.

/// One colour palette. Values are 0xRRGGBB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub window: u32,
    pub sidebar: u32,
    pub surface: u32,
    pub subtle: u32,
    pub selected: u32,
    pub fg: u32,
    pub fg2: u32,
    pub fg3: u32,
    pub border: u32,
    pub border_strong: u32,
    pub accent: u32,
    pub on_accent: u32,
    pub success: u32,
    pub success_bg: u32,
    pub warning: u32,
    pub warning_bg: u32,
    pub danger: u32,
    pub danger_bg: u32,
    pub danger_btn: u32,
    pub on_danger: u32,
    pub info: u32,
    pub info_bg: u32,
    pub add_bg: u32,
    pub del_bg: u32,
}

/// Forge Light. Primary blue #0A5BD3 (user decision 2026-11-14).
pub const LIGHT: Palette = Palette {
    window: 0xFFFFFF,
    sidebar: 0xF4F4F6,
    surface: 0xFFFFFF,
    subtle: 0xF0F0F3,
    selected: 0xE3ECFA,
    fg: 0x1C1C1F,
    fg2: 0x55555E,
    fg3: 0x66666F,
    border: 0xD6D6DC,
    border_strong: 0x8C8C96,
    accent: 0x0A5BD3,
    on_accent: 0xFFFFFF,
    success: 0x1A7431,
    success_bg: 0xE9F6ED,
    warning: 0x8A5300,
    warning_bg: 0xFFF5E0,
    danger: 0xC01F17,
    danger_bg: 0xFDEDEC,
    danger_btn: 0xC01F17,
    on_danger: 0xFFFFFF,
    info: 0x0B63A8,
    info_bg: 0xE8F1FB,
    add_bg: 0xE6F4EA,
    del_bg: 0xFBEAEA,
};

/// Forge Dark. Primary blue #5AA2FF.
pub const DARK: Palette = Palette {
    window: 0x1B1B1D,
    sidebar: 0x222225,
    surface: 0x2A2A2E,
    subtle: 0x303035,
    selected: 0x22344F,
    fg: 0xF2F2F5,
    fg2: 0xB0B0BA,
    fg3: 0x9A9AA5,
    border: 0x3E3E45,
    border_strong: 0x7C7C88,
    accent: 0x5AA2FF,
    on_accent: 0x0B1A2E,
    success: 0x5BCB7A,
    success_bg: 0x1F3326,
    warning: 0xF0B954,
    warning_bg: 0x3A3020,
    danger: 0xFF7A70,
    danger_bg: 0x3A2224,
    danger_btn: 0xC8342B,
    on_danger: 0xFFFFFF,
    info: 0x62B0F5,
    info_bg: 0x1F2D40,
    add_bg: 0x1E3324,
    del_bg: 0x3D2225,
};

impl Palette {
    pub fn for_dark(dark: bool) -> &'static Palette {
        if dark { &DARK } else { &LIGHT }
    }

    /// Looks up a token by its spec name (`fg2`, `borderStrong`, ...).
    pub fn token(&self, name: &str) -> Option<u32> {
        Some(match name {
            "window" => self.window,
            "sidebar" => self.sidebar,
            "surface" => self.surface,
            "subtle" => self.subtle,
            "selected" => self.selected,
            "fg" => self.fg,
            "fg2" => self.fg2,
            "fg3" => self.fg3,
            "border" => self.border,
            "borderStrong" => self.border_strong,
            "accent" => self.accent,
            "onAccent" => self.on_accent,
            "success" => self.success,
            "successBg" => self.success_bg,
            "warning" => self.warning,
            "warningBg" => self.warning_bg,
            "danger" => self.danger,
            "dangerBg" => self.danger_bg,
            "dangerBtn" => self.danger_btn,
            "onDanger" => self.on_danger,
            "info" => self.info,
            "infoBg" => self.info_bg,
            "addBg" => self.add_bg,
            "delBg" => self.del_bg,
            _ => return None,
        })
    }
}

/// gpui-kit theme colour key → Forge token (spec §3.1 right column). Hover and
/// active states reuse the base colour so no unlisted contrast pair appears.
pub const GPUI_KIT_MAPPING: &[(&str, &str)] = &[
    ("background", "window"),
    ("list.background", "window"),
    ("table.background", "window"),
    ("list.even.background", "window"),
    ("table.even.background", "window"),
    ("sidebar.background", "sidebar"),
    ("title_bar.background", "sidebar"),
    ("status_bar.background", "sidebar"),
    ("list.head.background", "sidebar"),
    ("table.head.background", "sidebar"),
    ("tab_bar.background", "sidebar"),
    ("tab.background", "sidebar"),
    ("popover.background", "surface"),
    ("group_box.background", "surface"),
    ("accordion.background", "surface"),
    ("button.background", "surface"),
    ("tab.active.background", "window"),
    ("muted.background", "subtle"),
    ("list.hover.background", "subtle"),
    ("table.hover.background", "subtle"),
    ("secondary.background", "subtle"),
    ("secondary.hover.background", "subtle"),
    ("skeleton.background", "subtle"),
    ("accent.background", "subtle"),
    ("button.hover.background", "subtle"),
    ("button.secondary.background", "subtle"),
    ("button.secondary.hover.background", "subtle"),
    ("tab_bar.segmented.background", "subtle"),
    ("list.active.background", "selected"),
    ("table.active.background", "selected"),
    ("sidebar.accent.background", "selected"),
    ("secondary.active.background", "selected"),
    ("selection.background", "selected"),
    ("drop_target.background", "selected"),
    ("button.active.background", "selected"),
    ("button.secondary.active.background", "selected"),
    ("foreground", "fg"),
    ("popover.foreground", "fg"),
    ("sidebar.foreground", "fg"),
    ("sidebar.accent.foreground", "fg"),
    ("group_box.foreground", "fg"),
    ("accent.foreground", "fg"),
    ("secondary.foreground", "fg"),
    ("button.foreground", "fg"),
    ("button.secondary.foreground", "fg"),
    ("tab.active.foreground", "fg"),
    ("muted.foreground", "fg2"),
    ("tab.foreground", "fg2"),
    ("table.head.foreground", "fg2"),
    ("description_list.label.foreground", "fg2"),
    ("group_box.title.foreground", "fg2"),
    ("border", "border"),
    ("sidebar.border", "border"),
    ("title_bar.border", "border"),
    ("status_bar.border", "border"),
    ("window.border", "border"),
    ("table.row.border", "border"),
    ("input.border", "borderStrong"),
    ("switch.background", "borderStrong"),
    ("scrollbar.thumb.background", "borderStrong"),
    ("scrollbar.thumb.hover.background", "fg2"),
    ("primary.background", "accent"),
    ("primary.hover.background", "accent"),
    ("primary.active.background", "accent"),
    ("ring", "accent"),
    ("link", "accent"),
    ("link.hover", "accent"),
    ("link.active", "accent"),
    ("list.active.border", "accent"),
    ("table.active.border", "accent"),
    ("caret", "accent"),
    ("drag.border", "accent"),
    ("progress.bar.background", "accent"),
    ("slider.background", "accent"),
    ("sidebar.primary.background", "accent"),
    ("button.primary.background", "accent"),
    ("button.primary.hover.background", "accent"),
    ("button.primary.active.background", "accent"),
    ("primary.foreground", "onAccent"),
    ("sidebar.primary.foreground", "onAccent"),
    ("button.primary.foreground", "onAccent"),
    ("slider.thumb.background", "window"),
    ("switch.thumb.background", "window"),
    ("danger.background", "dangerBtn"),
    ("danger.hover.background", "dangerBtn"),
    ("danger.active.background", "dangerBtn"),
    ("danger.foreground", "onDanger"),
    ("button.danger.background", "dangerBtn"),
    ("button.danger.hover.background", "dangerBtn"),
    ("button.danger.active.background", "dangerBtn"),
    ("button.danger.foreground", "onDanger"),
    ("success.background", "success"),
    ("success.hover.background", "success"),
    ("success.active.background", "success"),
    ("success.foreground", "successBg"),
    ("warning.background", "warning"),
    ("warning.hover.background", "warning"),
    ("warning.active.background", "warning"),
    ("warning.foreground", "warningBg"),
    ("info.background", "info"),
    ("info.hover.background", "info"),
    ("info.active.background", "info"),
    ("info.foreground", "infoBg"),
];

fn channel(value: u32, shift: u32) -> f64 {
    let c = ((value >> shift) & 0xFF) as f64 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG 2.x relative luminance.
pub fn luminance(rgb: u32) -> f64 {
    0.2126 * channel(rgb, 16) + 0.7152 * channel(rgb, 8) + 0.0722 * channel(rgb, 0)
}

/// WCAG 2.x contrast ratio (≥ 1.0).
pub fn contrast(a: u32, b: u32) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// The §3.1 contrast table, copied verbatim: (use, foreground token,
/// background token, minimum, spec light ratio, spec dark ratio).
pub const CONTRAST_PAIRS: &[(&str, &str, &str, f64, f64, f64)] = &[
    ("正文", "fg", "window", 4.5, 17.00, 15.39),
    ("侧栏正文", "fg", "sidebar", 4.5, 15.48, 14.20),
    ("卡片正文", "fg", "surface", 4.5, 17.00, 12.79),
    ("代码块/输出", "fg", "subtle", 4.5, 14.95, 11.75),
    ("选中行", "fg", "selected", 4.5, 14.28, 11.24),
    ("次要文字", "fg2", "window", 4.5, 7.38, 8.00),
    ("侧栏次要", "fg2", "sidebar", 4.5, 6.71, 7.38),
    ("卡片次要", "fg2", "surface", 4.5, 7.38, 6.65),
    ("输出次要", "fg2", "subtle", 4.5, 6.48, 6.10),
    ("选中行次要", "fg2", "selected", 4.5, 6.20, 5.84),
    ("占位/时间戳", "fg3", "window", 4.5, 5.68, 6.17),
    ("侧栏时间戳", "fg3", "sidebar", 4.5, 5.17, 5.70),
    ("输出占位", "fg3", "subtle", 4.5, 5.00, 4.71),
    ("选中行时间戳", "fg3", "selected", 4.5, 4.78, 4.51),
    ("链接", "accent", "window", 4.5, 6.09, 6.59),
    ("卡片链接", "accent", "surface", 4.5, 6.09, 5.48),
    ("选中行链接", "accent", "selected", 4.5, 5.11, 4.81),
    ("主按钮文字", "onAccent", "accent", 4.5, 6.09, 6.70),
    ("危险按钮文字", "onDanger", "dangerBtn", 4.5, 6.09, 5.27),
    ("成功文字", "success", "window", 4.5, 5.86, 8.41),
    ("成功底色上", "success", "successBg", 4.5, 5.26, 6.58),
    ("警告文字", "warning", "window", 4.5, 6.33, 9.64),
    ("警告底色上", "warning", "warningBg", 4.5, 5.84, 7.25),
    ("错误文字", "danger", "window", 4.5, 6.09, 6.77),
    ("卡片错误", "danger", "surface", 4.5, 6.09, 5.63),
    ("错误底色上", "danger", "dangerBg", 4.5, 5.36, 5.77),
    ("信息底色上", "info", "infoBg", 4.5, 5.47, 6.00),
    ("diff 新增行", "fg", "addBg", 4.5, 14.97, 12.09),
    ("diff 删除行", "fg", "delBg", 4.5, 14.62, 12.93),
    ("错误卡片正文", "fg", "dangerBg", 4.5, 14.98, 13.10),
    ("控件边框/图标", "borderStrong", "window", 3.0, 3.33, 4.17),
    ("卡片内控件边框", "borderStrong", "surface", 3.0, 3.33, 3.47),
    ("焦点环", "accent", "surface", 3.0, 6.09, 5.48),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_pair_meets_its_minimum() {
        for palette in [&LIGHT, &DARK] {
            for (name, fg, bg, min, _, _) in CONTRAST_PAIRS {
                let ratio = contrast(palette.token(fg).unwrap(), palette.token(bg).unwrap());
                assert!(ratio >= *min, "{name}: {fg}/{bg} = {ratio:.2} < {min}");
            }
        }
    }

    /// Our WCAG implementation reproduces every ratio printed in §3.1.
    #[test]
    fn matches_spec_ratios() {
        let round = |r: f64| (r * 100.0).round() / 100.0;
        for (name, fg, bg, _, light, dark) in CONTRAST_PAIRS {
            for (palette, expected) in [(&LIGHT, light), (&DARK, dark)] {
                let ratio = contrast(palette.token(fg).unwrap(), palette.token(bg).unwrap());
                assert!(
                    (round(ratio) - expected).abs() < 0.011,
                    "{name}: {fg}/{bg} = {ratio:.3}, spec says {expected}"
                );
            }
        }
    }

    /// Every gpui-kit foreground/background pairing the mapping creates (text
    /// colour on its own background key) is one of the listed §3.1 pairs.
    #[test]
    fn mapped_text_pairs_are_in_the_contrast_table() {
        let pairs = [
            ("primary.foreground", "primary.background"),
            ("button.primary.foreground", "button.primary.background"),
            ("danger.foreground", "danger.background"),
            ("button.danger.foreground", "button.danger.background"),
            ("success.foreground", "success.background"),
            ("warning.foreground", "warning.background"),
            ("info.foreground", "info.background"),
            ("foreground", "background"),
            ("sidebar.foreground", "sidebar.background"),
            ("sidebar.accent.foreground", "sidebar.accent.background"),
            ("popover.foreground", "popover.background"),
            ("muted.foreground", "background"),
            ("secondary.foreground", "secondary.background"),
        ];
        let token = |key: &str| {
            GPUI_KIT_MAPPING
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, t)| *t)
                .unwrap_or_else(|| panic!("{key} not mapped"))
        };
        for (fg, bg) in pairs {
            let (f, b) = (token(fg), token(bg));
            let listed = CONTRAST_PAIRS
                .iter()
                .any(|(_, x, y, ..)| (*x == f && *y == b) || (*x == b && *y == f));
            assert!(listed, "{fg}/{bg} → {f}/{b} is not in the §3.1 table");
        }
    }
}
