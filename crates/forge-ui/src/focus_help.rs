//! Visible shortcut help while a control holds keyboard focus (visual v2.1
//! §0 "键盘聚焦与鼠标悬停均应可查看", §4 "键盘聚焦时也能看到 tooltip").
//!
//! gpui-kit's managed tooltips are pointer-only, so every trigger that moved
//! information into a tooltip also anchors a floating chip to itself while
//! it — or, for the composer, its text field — holds keyboard focus. The
//! chip renders through GPUI's popover primitives (`deferred` +
//! `anchored().attach()`): it claims no layout space, flips to the other
//! side of the trigger near a window edge, carries no hitbox (pointer events
//! pass through) and never takes focus. Escape dismisses it until the
//! trigger blurs; blur re-arms it.

use crate::tokens::{self, Palette};
use gpui_kit::base::{ElementExt, TestSupportExt, TestTextExt};
use gpui_kit::component::ActiveTheme;
use gpui_kit::{
    Anchor, AnyElement, App, Bounds, ElementId, FocusHandle, InteractiveElement, IntoElement,
    ParentElement, Pixels, Role, SharedString, StatefulInteractiveElement, Styled, Window,
    anchored, deferred, div, point, prelude::FluentBuilder, px, rgb,
};
use std::cell::Cell;
use std::rc::Rc;

/// Per-trigger state, kept alive across frames by `use_keyed_state` and
/// dropped automatically once the trigger stops rendering.
#[derive(Clone)]
pub struct FocusHelp {
    /// Focus scope wrapped around the trigger (`track_focus`):
    /// `contains_focused` is true while the trigger holds keyboard focus.
    /// The wrapper itself never joins Tab order.
    pub scope: FocusHandle,
    /// Last prepainted trigger bounds — the anchor target for the chip.
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Escape dismissed the chip; a blur of `scope` re-arms it.
    pub dismissed: Rc<Cell<bool>>,
}

impl FocusHelp {
    /// `true` while the trigger's focus scope contains keyboard focus.
    pub fn focused(&self, window: &Window, cx: &App) -> bool {
        self.scope.contains_focused(window, cx)
    }

    /// Bookkeeping shared by every call site: `triggered` is the full show
    /// condition (the composer adds its text field). Resets the Escape
    /// dismissal once the condition lapses, then reports whether the chip
    /// should paint now.
    pub fn showing(&self, triggered: bool) -> bool {
        if !triggered {
            self.dismissed.set(false);
        }
        triggered && !self.dismissed.get()
    }

    /// Element action callback shared by the Escape bindings
    /// (`input::Escape` / `CloseSettings`): dismiss the chip and let the
    /// event keep bubbling so every other Escape behaviour is preserved.
    pub fn dismiss(&self, window: &mut Window, cx: &mut App) {
        self.dismissed.set(true);
        window.refresh();
        cx.propagate();
    }
}

/// Fetch (or create) the `FocusHelp` state keyed to `key`.
pub fn state(window: &mut Window, cx: &mut App, key: impl Into<ElementId>) -> FocusHelp {
    window
        .use_keyed_state(key, cx, |_, cx| FocusHelp {
            scope: cx.focus_handle(),
            bounds: Rc::new(Cell::new(None)),
            dismissed: Rc::new(Cell::new(false)),
        })
        .read(cx)
        .clone()
}

/// Wrap `trigger` so the chip can anchor to it: the wrapper tracks the help
/// scope (it stays out of Tab order itself and shrinks to the trigger's
/// bounds) and records those bounds each prepaint. `bubble` is rendered
/// beside it — deferred, so it neither sizes the wrapper nor clips to any
/// ancestor.
pub fn anchor(
    help: &FocusHelp,
    trigger: impl IntoElement,
    bubble: Option<AnyElement>,
) -> impl IntoElement {
    let scope = help.scope.clone();
    let bounds = help.bounds.clone();
    div()
        .track_focus(&scope)
        .tab_stop(false)
        .on_prepaint(move |b, _, _| bounds.set(Some(b)))
        .child(trigger)
        .children(bubble)
}

/// The floating help chip while `show` is true and the trigger's bounds are
/// known; `None` paints nothing and holds no layout space. Positioned like
/// a tooltip: centered above the trigger, `GAP` off its top edge.
pub fn bubble(
    help: &FocusHelp,
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    show: bool,
    palette: &Palette,
    cx: &App,
) -> Option<AnyElement> {
    if !show {
        return None;
    }
    let bounds = help.bounds.get()?;
    let text: SharedString = text.into();
    let dark = cx.theme().is_dark();
    let shadows = cx.theme().shadow;
    use tokens::component::help as hc;
    Some(
        deferred(
            anchored()
                .anchor(Anchor::BottomCenter)
                .attach(bounds, Anchor::TopCenter)
                .offset(point(px(0.), px(-hc::GAP)))
                .child(
                    div()
                        .id(id)
                        .role(Role::Tooltip)
                        .flex_shrink_0()
                        .px(px(hc::PAD_X))
                        .py(px(hc::PAD_Y))
                        .rounded(px(hc::RADIUS))
                        .bg(rgb(hc::bg(palette)))
                        .border_1()
                        .border_color(rgb(hc::border(palette)))
                        .text_size(px(tokens::CAPTION.size))
                        .line_height(px(tokens::CAPTION.line_height))
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(rgb(hc::fg(palette)))
                        .when(shadows, |el| el.shadow(crate::style::shadow_sm(dark)))
                        .child(div().test_text(text.clone()).child(text))
                        .test_support(),
                ),
        )
        .into_any_element(),
    )
}
