//! Shared control styling rules from §3.1 and the visual v2 elevation /
//! type-role helpers.

use crate::tokens::Palette;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::Button;
use gpui_kit::{App, BoxShadow, Div, ParentElement, Styled, div, hsla, px, rgb};

/// §3.1 rule 3: a disabled control keeps readable text — `fg3` on `subtle`
/// (5.00:1 light / 4.71:1 dark) with a `borderStrong` outline — and signals
/// "disabled" through semantics plus no hover/pointer (gpui-kit drops both for
/// disabled buttons), never through lowered contrast. gpui-kit's own disabled
/// style fades text to `muted_foreground` at 50% alpha, so it is overridden.
pub fn disabled_readable(button: Button, disabled: bool, cx: &App) -> Button {
    if !disabled {
        return button;
    }
    let palette = Palette::for_dark(cx.theme().is_dark());
    button
        .bg(rgb(palette.subtle))
        .text_color(rgb(palette.fg3))
        .border_1()
        .border_color(rgb(palette.subtle))
}

/// v2 `shadow-sm` (Composer): 0 1px 2px α4% + 0 2px 8px α6% — the dark theme
/// lifts the two layers to 30% / 25%.
pub fn shadow_sm(dark: bool) -> Vec<BoxShadow> {
    let (a, b) = if dark { (0.30, 0.25) } else { (0.04, 0.06) };
    vec![
        BoxShadow::new(px(0.), px(1.), hsla(0., 0., 0., a)).blur_radius(px(2.)),
        BoxShadow::new(px(0.), px(2.), hsla(0., 0., 0., b)).blur_radius(px(8.)),
    ]
}

/// v2 `shadow-lg` (dialogs): light 0 12px 32px −8px α18% + 0 4px 8px −4px α8%;
/// dark one deep layer 0 16px 40px −8px α60% (plus a 1pt border, set by the
/// dialog itself).
pub fn shadow_lg(dark: bool) -> Vec<BoxShadow> {
    if dark {
        vec![
            BoxShadow::new(px(0.), px(16.), hsla(0., 0., 0., 0.60))
                .blur_radius(px(40.))
                .spread_radius(px(-8.)),
        ]
    } else {
        vec![
            BoxShadow::new(px(0.), px(12.), hsla(0., 0., 0., 0.18))
                .blur_radius(px(32.))
                .spread_radius(px(-8.)),
            BoxShadow::new(px(0.), px(4.), hsla(0., 0., 0., 0.08))
                .blur_radius(px(8.))
                .spread_radius(px(-4.)),
        ]
    }
}

/// v2 composer focus glow: `0 0 0 3px` accent at 16% — a spread ring with no
/// blur or offset, layered on top of `shadow_sm`.
pub fn focus_glow(accent: u32) -> BoxShadow {
    BoxShadow::new(px(0.), px(0.), rgb(accent).opacity(0.16).into()).spread_radius(px(3.))
}

/// Kbd-style shortcut chip (`⌥⌘1`) in `fg3` (2026-11-14 Inspector tabs).
/// The glyphs are fixed macOS notation, like the other hints in the app.
pub fn kbd(keys: &str, palette: &Palette) -> Div {
    div()
        .px(px(4.))
        .rounded(px(4.))
        .border_1()
        .border_color(rgb(palette.border))
        .text_xs()
        .text_color(rgb(palette.fg3))
        .child(keys.to_owned())
}

/// V6 / UI-REF-04: every digit in the app is tabular (`tnum`), so counters,
/// run timers and countdowns ("0:09" → "0:10", "1 分 40 秒" → "1 分 39 秒")
/// never shift the text next to them. Set once on the window root and
/// inherited by all text.
pub fn tabular_figures() -> gpui_kit::FontFeatures {
    gpui_kit::FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)]))
}

#[cfg(test)]
mod tests {
    #[test]
    fn root_text_uses_tabular_figures() {
        let features = super::tabular_figures();
        assert!(features.0.iter().any(|(tag, on)| tag == "tnum" && *on == 1));
    }
}
