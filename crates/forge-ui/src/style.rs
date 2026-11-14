//! Shared control styling rules from §3.1.

use crate::tokens::Palette;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::Button;
use gpui_kit::{App, Div, ParentElement, Styled, div, px, rgb};

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
