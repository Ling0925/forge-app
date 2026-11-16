//! Forge design tokens (FORGE_APP_UI_SPEC.md §3.1 + visual v2 / v2.1) and the
//! WCAG contrast check behind the §3.1 table. Pure Rust so it is unit-tested
//! on Linux; the gpui-kit theme JSON in `themes/forge.json` is checked against
//! these values.
//!
//! v2.1 layers the tokens in three tiers (design/TOKENS_V2_1.md §1): the
//! *base* tier is the raw palette, type scale and `SPACE_*` / `RADIUS_*`
//! scales; the *semantic* tier is [`Palette`]'s fields (`sidebar`, `surface`,
//! `hover`, `selected`, `bubble`, `chip`, `dangerSoft`, `border`,
//! `borderStrong`, `accent`, `fg1-3`, `danger`, `success`); the *component*
//! tier in [`component`] binds semantics to named parts (`composer.radius`,
//! `card.error.bar`, ...). View code should reach for the last two tiers, not
//! the raw values.

/// One colour palette (semantic tier). Values are 0xRRGGBB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub window: u32,
    pub sidebar: u32,
    pub surface: u32,
    pub subtle: u32,
    /// v2: row / suggestion hover fill.
    pub hover: u32,
    pub selected: u32,
    /// v2: user message bubble.
    pub bubble: u32,
    /// v2: suggestion rows, workspace/model chips.
    pub chip: u32,
    /// v2: hovered chip / suggestion row.
    pub chip_hover: u32,
    /// v2: error card fill (softer than `danger_bg`).
    pub danger_soft: u32,
    pub fg: u32,
    pub fg2: u32,
    pub fg3: u32,
    /// Decorative hairlines only; controls use `border_strong` (≥3:1).
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

/// Modal scrim, light theme: black at 30 % (0x4D / 255). 0xRRGGBBAA.
pub const SCRIM_LIGHT: u32 = 0x0000_004D;
/// Modal scrim, dark theme: black at 50 % (0x80 / 255). 0xRRGGBBAA.
pub const SCRIM_DARK: u32 = 0x0000_0080;

/// Modal scrim (settings panel) as 0xRRGGBBAA for the theme; carried by the
/// gpui-kit `overlay` theme key.
pub const fn scrim(dark: bool) -> u32 {
    if dark { SCRIM_DARK } else { SCRIM_LIGHT }
}

/// Forge Light (visual v2). Primary blue #0A5BD3 (user decision 2026-11-14).
pub const LIGHT: Palette = Palette {
    window: 0xFFFFFF,
    sidebar: 0xF7F7F8,
    surface: 0xFFFFFF,
    subtle: 0xF0F0F3,
    hover: 0xEFEFF2,
    selected: 0xE9E9EE,
    bubble: 0xF2F2F5,
    chip: 0xF4F4F6,
    chip_hover: 0xEAEAEE,
    danger_soft: 0xFDF3F2,
    fg: 0x1C1C1F,
    fg2: 0x55555E,
    fg3: 0x66666F,
    border: 0xE4E4E8,
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

/// Forge Dark (visual v2). Primary blue #5AA2FF.
pub const DARK: Palette = Palette {
    window: 0x1B1B1D,
    sidebar: 0x202023,
    surface: 0x26262A,
    subtle: 0x303035,
    hover: 0x2A2A2E,
    selected: 0x2F2F34,
    bubble: 0x2C2C31,
    chip: 0x26262A,
    chip_hover: 0x303035,
    danger_soft: 0x2E2224,
    fg: 0xF2F2F5,
    fg2: 0xB0B0BA,
    fg3: 0x9A9AA5,
    border: 0x34343A,
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

    /// Looks up a token by its spec name (`fg2`, `borderStrong`, `chipHover`,
    /// ...). The v2 semantic name `fg1` aliases `fg`.
    pub fn token(&self, name: &str) -> Option<u32> {
        Some(match name {
            "window" => self.window,
            "sidebar" => self.sidebar,
            "surface" => self.surface,
            "subtle" => self.subtle,
            "hover" => self.hover,
            "selected" => self.selected,
            "bubble" => self.bubble,
            "chip" => self.chip,
            "chipHover" => self.chip_hover,
            "dangerSoft" => self.danger_soft,
            "fg" | "fg1" => self.fg,
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

/// gpui-kit theme colour key → Forge token (spec §3.1 right column + v2 row
/// hover). Hover on rows/lists uses the `hover` token; hover on button-type
/// controls reuses `subtle` so no unlisted contrast pair appears.
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
    ("secondary.background", "subtle"),
    ("secondary.hover.background", "subtle"),
    ("skeleton.background", "subtle"),
    ("accent.background", "subtle"),
    ("button.hover.background", "subtle"),
    ("button.secondary.background", "subtle"),
    ("button.secondary.hover.background", "subtle"),
    ("tab_bar.segmented.background", "subtle"),
    // v2: row / suggestion-row hover.
    ("list.hover.background", "hover"),
    ("table.hover.background", "hover"),
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

/// The §3.1 contrast table extended for the v2 combinations (design
/// `contrast-v2.txt`): (use, foreground token, background token, minimum,
/// measured light ratio, measured dark ratio).
pub const CONTRAST_PAIRS: &[(&str, &str, &str, f64, f64, f64)] = &[
    ("正文", "fg", "window", 4.5, 17.00, 15.39),
    ("侧栏正文", "fg", "sidebar", 4.5, 15.88, 14.54),
    ("卡片正文", "fg", "surface", 4.5, 17.00, 13.49),
    ("代码块/输出", "fg", "subtle", 4.5, 14.95, 11.75),
    ("选中行", "fg", "selected", 4.5, 14.05, 11.92),
    ("行悬停", "fg", "hover", 4.5, 14.81, 12.79),
    ("用户气泡正文", "fg", "bubble", 4.5, 15.22, 12.43),
    ("建议行正文", "fg", "chip", 4.5, 15.48, 13.49),
    ("建议行悬停", "fg", "chipHover", 4.5, 14.17, 11.75),
    ("错误卡片正文", "fg", "dangerSoft", 4.5, 15.61, 13.71),
    ("次要文字", "fg2", "window", 4.5, 7.38, 8.00),
    ("侧栏次要", "fg2", "sidebar", 4.5, 6.89, 7.56),
    ("卡片次要", "fg2", "surface", 4.5, 7.38, 7.01),
    ("输出次要", "fg2", "subtle", 4.5, 6.48, 6.10),
    ("选中行次要", "fg2", "selected", 4.5, 6.10, 6.19),
    ("行悬停次要", "fg2", "hover", 4.5, 6.43, 6.65),
    ("用户气泡次要", "fg2", "bubble", 4.5, 6.60, 6.46),
    ("建议行次要", "fg2", "chip", 4.5, 6.71, 7.01),
    ("错误卡片次要", "fg2", "dangerSoft", 4.5, 6.77, 7.13),
    ("占位/时间戳", "fg3", "window", 4.5, 5.68, 6.17),
    ("侧栏时间戳", "fg3", "sidebar", 4.5, 5.31, 5.83),
    ("卡片占位", "fg3", "surface", 4.5, 5.68, 5.41),
    ("输出占位", "fg3", "subtle", 4.5, 5.00, 4.71),
    ("禁用按钮文字", "fg3", "subtle", 4.5, 5.00, 4.71),
    ("选中行时间戳", "fg3", "selected", 4.5, 4.70, 4.78),
    ("行悬停占位", "fg3", "hover", 4.5, 4.95, 5.13),
    ("气泡占位", "fg3", "bubble", 4.5, 5.09, 4.99),
    ("建议行占位", "fg3", "chip", 4.5, 5.17, 5.41),
    ("建议行悬停占位", "fg3", "chipHover", 4.5, 4.74, 4.71),
    ("链接", "accent", "window", 4.5, 6.09, 6.59),
    ("卡片链接", "accent", "surface", 4.5, 6.09, 5.78),
    ("选中行链接", "accent", "selected", 4.5, 5.03, 5.10),
    ("主按钮文字", "onAccent", "accent", 4.5, 6.09, 6.70),
    ("危险按钮文字", "onDanger", "dangerBtn", 4.5, 6.09, 5.27),
    ("成功文字", "success", "window", 4.5, 5.86, 8.41),
    ("成功底色上", "success", "successBg", 4.5, 5.26, 6.58),
    ("设置卡片成功", "success", "sidebar", 4.5, 5.47, 7.95),
    ("设置卡片成功深", "success", "selected", 4.5, 4.84, 6.51),
    ("警告文字", "warning", "window", 4.5, 6.33, 9.64),
    ("警告底色上", "warning", "warningBg", 4.5, 5.84, 7.25),
    ("错误文字", "danger", "window", 4.5, 6.09, 6.77),
    ("卡片错误", "danger", "surface", 4.5, 6.09, 5.94),
    ("错误底色上", "danger", "dangerBg", 4.5, 5.36, 5.77),
    ("错误卡片标题/竖条", "danger", "dangerSoft", 4.5, 5.59, 6.03),
    ("设置卡片错误", "danger", "sidebar", 4.5, 5.69, 6.40),
    ("设置卡片错误深", "danger", "selected", 4.5, 5.03, 5.24),
    ("信息文字", "info", "window", 4.5, 6.24, 7.41),
    ("信息底色上", "info", "infoBg", 4.5, 5.47, 6.00),
    ("diff 新增行", "fg", "addBg", 4.5, 14.97, 12.09),
    ("diff 删除行", "fg", "delBg", 4.5, 14.62, 12.93),
    ("错误卡旧底色正文", "fg", "dangerBg", 4.5, 14.98, 13.10),
    ("控件边框/图标", "borderStrong", "window", 3.0, 3.33, 4.17),
    ("卡片内控件边框", "borderStrong", "surface", 3.0, 3.33, 3.66),
    ("焦点环", "accent", "surface", 3.0, 6.09, 5.78),
];

/// Type scale at 100% (spec §3.3 + visual v2), sizes and line-heights in
/// points. v2 drops 11pt: every role is ≥12.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypeRole {
    pub size: f32,
    pub line_height: f32,
    pub weight: TypeWeight,
}

/// Font weight per §3.3; mapped to `gpui_kit::FontWeight` at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeWeight {
    Regular,
    Medium,
    Semibold,
}

pub const TITLE1: TypeRole = TypeRole {
    size: 22.0,
    line_height: 28.0,
    weight: TypeWeight::Semibold,
};
pub const TITLE2: TypeRole = TypeRole {
    size: 17.0,
    line_height: 22.0,
    weight: TypeWeight::Semibold,
};
pub const TITLE3: TypeRole = TypeRole {
    size: 15.0,
    line_height: 20.0,
    weight: TypeWeight::Semibold,
};
/// v2 `message`: 对话正文、用户气泡.
pub const MESSAGE: TypeRole = TypeRole {
    size: 14.0,
    line_height: 22.0,
    weight: TypeWeight::Regular,
};
/// v2 `callout`: Composer 输入.
pub const CALLOUT: TypeRole = TypeRole {
    size: 14.0,
    line_height: 21.0,
    weight: TypeWeight::Regular,
};
pub const BODY: TypeRole = TypeRole {
    size: 13.0,
    line_height: 20.0,
    weight: TypeWeight::Regular,
};
/// v2 `bodyStrong`: same size as `body`, Semibold.
pub const BODY_STRONG: TypeRole = TypeRole {
    size: 13.0,
    line_height: 20.0,
    weight: TypeWeight::Semibold,
};
pub const FOOTNOTE: TypeRole = TypeRole {
    size: 12.0,
    line_height: 16.0,
    weight: TypeWeight::Regular,
};
/// v2: 12/16 Medium (was 11/14); 分组标题、角标、快捷键提示.
pub const CAPTION: TypeRole = TypeRole {
    size: 12.0,
    line_height: 16.0,
    weight: TypeWeight::Medium,
};
pub const MONO: TypeRole = TypeRole {
    size: 12.0,
    line_height: 18.0,
    weight: TypeWeight::Regular,
};

/// v2.1 spacing scale (4pt multiples only, §2). Strokes — 1pt/1.5pt borders,
/// the 3pt glow and the error bar — are not spacing.
pub const SPACE_1: f32 = 4.0;
pub const SPACE_2: f32 = 8.0;
pub const SPACE_3: f32 = 12.0;
pub const SPACE_4: f32 = 16.0;
pub const SPACE_5: f32 = 20.0;
pub const SPACE_6: f32 = 24.0;
pub const SPACE_8: f32 = 32.0;
pub const SPACE_12: f32 = 48.0;

/// v2.1 radius scale (§3): 8 / 12 / 16 / 24 / full. The old 10 and 18 are
/// gone. Nested radius rule: inner = outer − padding, floored at 4.
pub const RADIUS_SM: f32 = 8.0;
pub const RADIUS_MD: f32 = 12.0;
pub const RADIUS_LG: f32 = 16.0;
pub const RADIUS_XL: f32 = 24.0;
/// Capsule / circular controls.
pub const RADIUS_FULL: f32 = 9999.0;

/// v2.1 nested-radius rule (§3): inner = outer − padding, minimum 4.
pub fn nested_radius(outer: f32, padding: f32) -> f32 {
    (outer - padding).max(4.0)
}

/// Component tier (v2.1 §1): named parts bound to semantic tokens. Changing
/// the look touches this layer and [`Palette`], never call-site literals.
pub mod component {
    use super::{
        Palette, RADIUS_LG, RADIUS_MD, RADIUS_SM, RADIUS_XL, SPACE_1, SPACE_2, SPACE_3, SPACE_4,
        SPACE_6, SPACE_8, SPACE_12, nested_radius,
    };

    /// Composer: floating surface, v2.1 §2/§4.
    pub mod composer {
        use super::*;
        /// Card radius (`radius-xl`).
        pub const RADIUS: f32 = RADIUS_XL;
        /// Text-area padding on all four sides (`space-4`).
        pub const PADDING: f32 = SPACE_4;
        /// Send/stop control inset from the card border (top/right/bottom).
        pub const CONTROL_INSET: f32 = SPACE_2;
        /// Distance from the window's bottom edge.
        pub const BOTTOM: f32 = SPACE_4;
        /// Resting border (1pt stroke) and focused border (1.5pt stroke).
        pub const BORDER: f32 = 1.0;
        pub const BORDER_FOCUS: f32 = 1.5;
        /// Focus glow spread (3pt stroke) at accent 16%.
        pub const FOCUS_GLOW: f32 = 3.0;
        /// 32pt circular send button (= 2 × `RADIUS_LG`, concentric:
        /// `RADIUS` − `CONTROL_INSET` = 16).
        pub const SEND_SIZE: f32 = 32.0;
        pub fn send_radius() -> f32 {
            super::nested_radius(RADIUS, CONTROL_INSET)
        }
        pub fn resting_border(p: &Palette) -> u32 {
            p.border_strong
        }
        pub fn focus_border(p: &Palette) -> u32 {
            p.accent
        }
        pub fn focus_glow(p: &Palette) -> u32 {
            p.accent
        }
        pub fn fill(p: &Palette) -> u32 {
            p.surface
        }
        pub fn send_bg(p: &Palette) -> u32 {
            p.accent
        }
        pub fn send_fg(p: &Palette) -> u32 {
            p.on_accent
        }
        /// Disabled send: readable `fg3` arrow on `subtle`, never faded.
        pub fn send_disabled_bg(p: &Palette) -> u32 {
            p.subtle
        }
        pub fn send_disabled_fg(p: &Palette) -> u32 {
            p.fg3
        }
        /// Workspace / model chips inside the composer toolbar.
        pub fn chip_bg(p: &Palette) -> u32 {
            p.chip
        }
        pub fn chip_fg(p: &Palette) -> u32 {
            p.fg2
        }
        /// Stop pill: neutral outline (`borderStrong`, `fg` text) — v2.1
        /// drops the v1 danger outline.
        pub fn stop_border(p: &Palette) -> u32 {
            p.border_strong
        }
        pub fn stop_fg(p: &Palette) -> u32 {
            p.fg
        }
        pub fn stop_hover(p: &Palette) -> u32 {
            p.chip_hover
        }
    }

    /// Cards (error, suggestion, settings groups): 16pt padding, radius-md,
    /// 8pt between cards (v2.1 §2).
    pub mod card {
        use super::*;
        pub const RADIUS: f32 = RADIUS_MD;
        pub const PADDING: f32 = SPACE_4;
        pub const GAP: f32 = SPACE_2;
        /// Error card: `dangerSoft` fill + 3pt `danger` left bar, no ring.
        pub mod error {
            use super::*;
            /// Left status bar width — a stroke, not spacing (§2 note).
            pub const BAR: f32 = 3.0;
            pub fn bg(p: &Palette) -> u32 {
                p.danger_soft
            }
            pub fn bar(p: &Palette) -> u32 {
                p.danger
            }
            pub fn title_fg(p: &Palette) -> u32 {
                p.danger
            }
            pub fn body_fg(p: &Palette) -> u32 {
                p.fg
            }
        }
        /// Empty-state suggestion rows: chip fill, chipHover on hover.
        pub mod suggestion {
            use super::*;
            pub fn bg(p: &Palette) -> u32 {
                p.chip
            }
            pub fn hover(p: &Palette) -> u32 {
                p.chip_hover
            }
        }
        /// Settings group card: `sidebar` (light) / `selected` (dark).
        pub fn settings_bg(p: &Palette, dark: bool) -> u32 {
            if dark { p.selected } else { p.sidebar }
        }
    }

    /// Settings / onboarding dialogs (`radius-xl`, 24pt padding, 24pt group
    /// gap, 8pt group title → card).
    pub mod dialog {
        use super::*;
        pub const RADIUS: f32 = RADIUS_XL;
        pub const PADDING: f32 = SPACE_6;
        pub const GROUP_GAP: f32 = SPACE_6;
        pub const GROUP_TITLE_GAP: f32 = SPACE_2;
        pub const WIDTH: f32 = 560.0;
        /// Hairline between rows inside a group card (decorative `border`).
        pub fn separator(p: &Palette) -> u32 {
            p.border
        }
        /// Dark dialogs keep a 1pt `border` ring under the shadow (v2 §3);
        /// increase-contrast swaps the shadow for `borderStrong` (§5.4).
        pub fn border_color(p: &Palette, dark: bool, shadows_on: bool) -> Option<u32> {
            if !shadows_on {
                Some(p.border_strong)
            } else if dark {
                Some(p.border)
            } else {
                None
            }
        }
    }

    /// Sidebar rows and controls.
    pub mod sidebar {
        use super::*;
        pub const ROW_RADIUS: f32 = RADIUS_SM;
        pub const ROW_HEIGHT: f32 = SPACE_12;
        /// "＋ 新会话" full-width row height (§0: 36 → 32).
        pub const NEW_SESSION_HEIGHT: f32 = SPACE_8;
        /// Inner horizontal padding (`space-3`).
        pub const ROW_PADDING: f32 = SPACE_3;
        /// Row-to-sidebar-edge gap (`space-2`).
        pub const EDGE: f32 = SPACE_2;
        pub fn selected_bg(p: &Palette) -> u32 {
            p.selected
        }
        pub fn hover_bg(p: &Palette) -> u32 {
            p.hover
        }
    }

    /// Plain buttons and inputs inside cards: radius-sm (12 − 4 = 8).
    pub mod button {
        use super::*;
        pub const RADIUS: f32 = RADIUS_SM;
        /// Compact control height (§0: the 28pt controls unify at 32) —
        /// error-card / settings buttons, key inputs, the ✕ close and the
        /// theme segmented track. This is a control height, not a new step on
        /// the spacing scale.
        pub const HEIGHT: f32 = SPACE_8;
    }

    /// Settings 主题 segmented control (§0: the ~28pt track unifies at 32):
    /// 32pt track with a 2pt inset → 28pt segments nesting at radius-sm − 2.
    pub mod segmented {
        use super::*;
        pub const TRACK_INSET: f32 = 2.0;
        pub const TRACK_RADIUS: f32 = RADIUS_SM;
        pub const TRACK_HEIGHT: f32 = button::HEIGHT;
        /// Segment height inside the track (32 − 2 × 2).
        pub const SEGMENT_HEIGHT: f32 = TRACK_HEIGHT - 2.0 * TRACK_INSET;
        pub fn segment_radius() -> f32 {
            super::nested_radius(TRACK_RADIUS, TRACK_INSET)
        }
    }

    /// User message bubble.
    pub mod bubble {
        use super::*;
        pub const RADIUS: f32 = RADIUS_LG;
        pub const PAD_X: f32 = SPACE_4;
        pub const PAD_Y: f32 = SPACE_2;
        pub fn bg(p: &Palette) -> u32 {
            p.bubble
        }
    }

    /// Timeline message spacing (v2.1 §2): 12 inside a turn, 24 between.
    pub mod timeline {
        use super::*;
        pub const ROW_GAP: f32 = SPACE_3;
        pub const TURN_GAP: f32 = SPACE_6;
    }

    /// Session header (顶栏): 52pt tall, borderless (§0 keeps the height,
    /// narrows the side padding 28 → 24).
    pub mod header {
        use super::*;
        pub const HEIGHT: f32 = 52.0;
        /// Horizontal padding (`space-6`).
        pub const PADDING_X: f32 = SPACE_6;
    }

    /// Empty screen (§0): the bottom padding lifts the column 60 → 48.
    pub mod empty_state {
        use super::*;
        pub const BOTTOM: f32 = SPACE_12;
    }

    /// Keyboard-focus shortcut help (§0: 键盘聚焦时也能看到 tooltip): a
    /// floating chip anchored to the focused control, painted by the
    /// deferred/anchored popover primitives so it never claims layout space.
    pub mod help {
        use super::*;
        /// Chip radius (`radius-sm`).
        pub const RADIUS: f32 = RADIUS_SM;
        /// Side padding (`space-2`).
        pub const PAD_X: f32 = SPACE_2;
        /// Vertical padding (`space-1`).
        pub const PAD_Y: f32 = SPACE_1;
        /// Gap between the trigger's top edge and the chip.
        pub const GAP: f32 = SPACE_1;
        pub fn bg(p: &Palette) -> u32 {
            p.surface
        }
        pub fn fg(p: &Palette) -> u32 {
            p.fg
        }
        pub fn border(p: &Palette) -> u32 {
            p.border_strong
        }
    }

    /// Cards are spaced 8pt apart in a group/stack.
    pub const CARD_GAP: f32 = SPACE_2;
    /// Content column (v2 §3): 720pt centred. Single source of truth:
    /// `forge_state::CONTENT_MAX`.
    pub const CONTENT_MAX: f32 = forge_state::CONTENT_MAX;
}

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

    /// Our WCAG implementation reproduces every measured v2 ratio.
    #[test]
    fn matches_spec_ratios() {
        let round = |r: f64| (r * 100.0).round() / 100.0;
        for (name, fg, bg, _, light, dark) in CONTRAST_PAIRS {
            for (palette, expected) in [(&LIGHT, light), (&DARK, dark)] {
                let ratio = contrast(palette.token(fg).unwrap(), palette.token(bg).unwrap());
                assert!(
                    (round(ratio) - expected).abs() < 0.011,
                    "{name}: {fg}/{bg} = {ratio:.3}, table says {expected}"
                );
            }
        }
    }

    #[test]
    fn token_lookup_covers_every_field() {
        let names = [
            "window",
            "sidebar",
            "surface",
            "subtle",
            "hover",
            "selected",
            "bubble",
            "chip",
            "chipHover",
            "dangerSoft",
            "fg",
            "fg1",
            "fg2",
            "fg3",
            "border",
            "borderStrong",
            "accent",
            "onAccent",
            "success",
            "successBg",
            "warning",
            "warningBg",
            "danger",
            "dangerBg",
            "dangerBtn",
            "onDanger",
            "info",
            "infoBg",
            "addBg",
            "delBg",
        ];
        for name in names {
            assert!(LIGHT.token(name).is_some(), "{name} missing");
        }
        for bad in ["", "fg4", "borderstrong", "Border", "accentBg"] {
            assert_eq!(LIGHT.token(bad), None, "{bad}");
        }
        // Every gpui-kit mapping value must resolve to a real token.
        for (key, token) in GPUI_KIT_MAPPING {
            assert!(
                LIGHT.token(token).is_some(),
                "{key} maps to unknown token {token}"
            );
        }
    }

    #[test]
    fn palette_selection_and_scrim() {
        assert_eq!(Palette::for_dark(true), &DARK);
        assert_eq!(Palette::for_dark(false), &LIGHT);
        assert_eq!(scrim(true), SCRIM_DARK);
        assert_eq!(scrim(false), SCRIM_LIGHT);
        // Scrim: black at 30% / 50% alpha (0xRRGGBBAA).
        assert_eq!(SCRIM_LIGHT & 0xFF, 0x4D);
        assert_eq!(SCRIM_DARK & 0xFF, 0x80);
        assert_eq!(SCRIM_LIGHT >> 8, 0);
        assert_eq!(SCRIM_DARK >> 8, 0);
    }

    /// v2.1 §2/§3: the type scale is ≥12pt and spacing/radii sit on their
    /// scales (spacing all multiples of 4).
    #[test]
    fn type_scale_and_spacing_match_spec() {
        let roles = [
            (TITLE1, 22.0, 28.0, TypeWeight::Semibold),
            (TITLE2, 17.0, 22.0, TypeWeight::Semibold),
            (TITLE3, 15.0, 20.0, TypeWeight::Semibold),
            (MESSAGE, 14.0, 22.0, TypeWeight::Regular),
            (CALLOUT, 14.0, 21.0, TypeWeight::Regular),
            (BODY, 13.0, 20.0, TypeWeight::Regular),
            (BODY_STRONG, 13.0, 20.0, TypeWeight::Semibold),
            (FOOTNOTE, 12.0, 16.0, TypeWeight::Regular),
            (CAPTION, 12.0, 16.0, TypeWeight::Medium),
            (MONO, 12.0, 18.0, TypeWeight::Regular),
        ];
        for (role, size, line_height, weight) in roles {
            let (size, line_height): (f32, f32) = (size, line_height);
            assert_eq!(role.size.to_bits(), size.to_bits());
            assert_eq!(role.line_height.to_bits(), line_height.to_bits());
            assert_eq!(role.weight, weight);
            // v2: nothing under 12pt.
            assert!(role.size >= 12.0);
            assert!(role.line_height > role.size);
        }
        let spacing = [
            SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6, SPACE_8, SPACE_12,
        ];
        let spec: [f32; 8] = [4.0, 8.0, 12.0, 16.0, 20.0, 24.0, 32.0, 48.0];
        assert_eq!(
            spacing.map(f32::to_bits).as_slice(),
            spec.map(f32::to_bits).as_slice()
        );
        assert!(spacing.iter().all(|v| v % 4.0 == 0.0));
        assert!(spacing.windows(2).all(|w| w[0] < w[1]));
        let radii = [RADIUS_SM, RADIUS_MD, RADIUS_LG, RADIUS_XL];
        assert_eq!(radii, [8.0, 12.0, 16.0, 24.0]);
        assert!(radii.iter().all(|v| v % 4.0 == 0.0));
        // Nested-radius rule: inner = outer − padding, floor 4.
        assert_eq!(nested_radius(24.0, 8.0), 16.0);
        assert_eq!(nested_radius(24.0, 12.0), 12.0);
        assert_eq!(nested_radius(12.0, 4.0), 8.0);
        assert_eq!(nested_radius(8.0, 6.0), 4.0);
    }

    /// v2.1 component tier: the nested-radius and spacing invariants hold.
    #[test]
    fn component_layer_follows_the_rules() {
        use component::*;
        // Composer ⊃ send button: 24 − 8 = 16 → the 32pt circle is concentric.
        assert_eq!(composer::send_radius(), 16.0);
        assert_eq!(composer::SEND_SIZE, 2.0 * composer::send_radius());
        assert_eq!(composer::RADIUS, RADIUS_XL);
        assert_eq!(composer::PADDING, SPACE_4);
        assert_eq!(composer::CONTROL_INSET, SPACE_2);
        assert_eq!(composer::BOTTOM, SPACE_4);
        // Dialog ⊃ group card is content layout, not a concentric outline
        // (§0): the card keeps its own radius-md 12 inside the 24pt dialog
        // padding. The nested-radius formula applies only to true outlines —
        // composer ⊃ send (above) and segmented track ⊃ segment (below).
        assert_eq!(dialog::GROUP_GAP, SPACE_6);
        assert_eq!(dialog::PADDING, SPACE_6);
        assert_eq!(dialog::WIDTH, 560.0);
        // §0 small controls unify at 32 — a control height, not a new step on
        // the spacing scale: card/settings buttons, key inputs, the ✕ close,
        // the theme segmented track, and the ＋ 新会话 row.
        assert_eq!(button::HEIGHT, 32.0);
        assert_eq!(segmented::TRACK_HEIGHT, button::HEIGHT);
        assert_eq!(segmented::SEGMENT_HEIGHT, 28.0);
        assert_eq!(segmented::segment_radius(), nested_radius(RADIUS_SM, 2.0));
        // Sidebar row, suggestion, error card, bubble radii.
        assert_eq!(sidebar::ROW_RADIUS, RADIUS_SM);
        assert_eq!(sidebar::ROW_HEIGHT, 48.0);
        assert_eq!(sidebar::NEW_SESSION_HEIGHT, 32.0);
        // §0 header: 52pt tall, 24pt side padding; the empty state column
        // lifts 60 → 48 off the bottom edge.
        assert_eq!(header::HEIGHT, 52.0);
        assert_eq!(header::PADDING_X, SPACE_6);
        assert_eq!(empty_state::BOTTOM, 48.0);
        assert_eq!(card::RADIUS, RADIUS_MD);
        assert_eq!(card::PADDING, SPACE_4);
        assert_eq!(card::error::BAR, 3.0);
        assert_eq!(bubble::RADIUS, RADIUS_LG);
        assert_eq!(timeline::ROW_GAP, 12.0);
        assert_eq!(timeline::TURN_GAP, 24.0);
        // Component colours bind to semantic tokens.
        for p in [&LIGHT, &DARK] {
            assert_eq!(composer::resting_border(p), p.border_strong);
            assert_eq!(composer::focus_border(p), p.accent);
            assert_eq!(composer::fill(p), p.surface);
            assert_eq!(card::error::bg(p), p.danger_soft);
            assert_eq!(card::error::bar(p), p.danger);
            assert_eq!(card::error::title_fg(p), p.danger);
            assert_eq!(card::suggestion::bg(p), p.chip);
            assert_eq!(card::suggestion::hover(p), p.chip_hover);
            assert_eq!(bubble::bg(p), p.bubble);
            assert_eq!(sidebar::selected_bg(p), p.selected);
        }
        // Settings card: sidebar (light) / selected (dark).
        assert_eq!(card::settings_bg(&LIGHT, false), LIGHT.sidebar);
        assert_eq!(card::settings_bg(&DARK, true), DARK.selected);
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
