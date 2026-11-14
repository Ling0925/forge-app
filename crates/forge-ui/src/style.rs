//! Shared control styling rules from §3.1.

use crate::tokens::Palette;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::Button;
use gpui_kit::{App, Styled, rgb};

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
        .border_color(rgb(palette.border_strong))
}
