//! Shared item card (spec "参考提取" V1, UI-REF-01): the one skeleton used by
//! tool-step (§6.5), approval (§6.6) and error (§6.7) cards.
//!
//! ```text
//! [icon]  (● 状态) 来源 · 时间                         [⋯]
//!         标题：结论 + 关键数字
//!         一行次要说明
//!         [情境主动作] [详情]
//!         (details)
//! ```
//!
//! Only the error card has data today; tool and approval cards arrive with
//! Core P2 and should be built on [`ItemCard`] too.

use crate::tokens::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, ClickEvent, InteractiveElement, IntoElement, ParentElement, Role,
    SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px, rgb,
};
use std::rc::Rc;

/// Colour family of a card. Every tone pairs a text-safe foreground with its
/// tinted background (all ≥4.5:1, see `tokens::CONTRAST_PAIRS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Info,
    Success,
    Warning,
    Danger,
}

impl Tone {
    /// (foreground, tinted background, card border)
    pub fn colors(self, p: &Palette) -> (u32, u32, u32) {
        match self {
            Self::Neutral => (p.fg2, p.subtle, p.border),
            Self::Info => (p.info, p.info_bg, p.border),
            Self::Success => (p.success, p.success_bg, p.border),
            Self::Warning => (p.warning, p.warning_bg, p.warning),
            Self::Danger => (p.danger, p.danger_bg, p.danger),
        }
    }
}

type MenuHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

pub struct ItemCard {
    id: SharedString,
    tone: Tone,
    icon: IconName,
    status: SharedString,
    meta: Option<SharedString>,
    title: SharedString,
    body: Option<SharedString>,
    a11y_label: SharedString,
    actions: Vec<AnyElement>,
    details: Option<AnyElement>,
    menu: Vec<(SharedString, MenuHandler)>,
}

impl ItemCard {
    pub fn new(
        id: impl Into<SharedString>,
        tone: Tone,
        icon: IconName,
        status: impl Into<SharedString>,
        title: impl Into<SharedString>,
    ) -> Self {
        let title = title.into();
        Self {
            id: id.into(),
            tone,
            icon,
            status: status.into(),
            meta: None,
            a11y_label: title.clone(),
            title,
            body: None,
            actions: Vec::new(),
            details: None,
            menu: Vec::new(),
        }
    }

    pub fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        let meta = meta.into();
        self.meta = (!meta.is_empty()).then_some(meta);
        self
    }

    pub fn body(mut self, body: impl Into<SharedString>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn a11y_label(mut self, label: impl Into<SharedString>) -> Self {
        self.a11y_label = label.into();
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    pub fn details(mut self, details: Option<AnyElement>) -> Self {
        self.details = details;
        self
    }

    /// Adds an entry to the top-right ⋯ overflow menu.
    pub fn menu_item(
        mut self,
        label: impl Into<SharedString>,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.menu.push((label.into(), Rc::new(handler)));
        self
    }

    pub fn render(self, palette: &Palette) -> AnyElement {
        let (tone_fg, tone_bg, border) = self.tone.colors(palette);
        let id = self.id.clone();
        // Status pill: dot + text on the card surface, so the text keeps
        // ≥4.5:1 whatever the card tint (UI-REF-02); colour is never alone.
        let pill = h_flex()
            .id(SharedString::from(format!("{id}-status")))
            .gap_1()
            .px_1p5()
            .rounded_full()
            .bg(rgb(palette.surface))
            .border_1()
            .border_color(rgb(tone_fg))
            .text_xs()
            .text_color(rgb(tone_fg))
            .child(div().size(px(6.)).rounded_full().bg(rgb(tone_fg)))
            .child(self.status.clone())
            .test_support();
        let overflow = (!self.menu.is_empty()).then(|| {
            let items = self.menu.clone();
            Button::new(SharedString::from(format!("{id}-more")))
                .icon(IconName::Ellipsis)
                .ghost()
                .xsmall()
                .accessibility_label(forge_state::i18n::t("card.more"))
                .tooltip(forge_state::i18n::t("card.more"))
                .dropdown_menu(move |mut menu, _, _| {
                    for (label, handler) in &items {
                        let handler = handler.clone();
                        menu = menu.item(
                            PopupMenuItem::new(label.clone())
                                .on_click(move |e, w, cx| handler(e, w, cx)),
                        );
                    }
                    menu
                })
        });
        let header = h_flex()
            .gap_2()
            .items_center()
            .child(pill)
            .children(
                self.meta
                    .map(|meta| div().text_xs().text_color(rgb(palette.fg2)).child(meta)),
            )
            .child(div().flex_1())
            .children(overflow);
        v_flex()
            .id(self.id)
            .role(Role::Group)
            .aria_label(self.a11y_label)
            .when_some(self.body.clone(), |el, desc| el.aria_description(desc))
            .p_4()
            .rounded(px(8.))
            .bg(rgb(tone_bg))
            .border_1()
            .border_color(rgb(border))
            .text_color(rgb(palette.fg))
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    // Type icon block.
                    .child(
                        div()
                            .flex_shrink_0()
                            .size(px(28.))
                            .rounded(px(6.))
                            .bg(rgb(palette.surface))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Icon::new(self.icon).text_color(rgb(tone_fg))),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(header)
                            .child(
                                div()
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .child(self.title),
                            )
                            .children(self.body.map(|body| {
                                div().text_sm().text_color(rgb(palette.fg)).child(body)
                            }))
                            .when(!self.actions.is_empty(), |col| {
                                col.child(h_flex().pt_1().gap_2().children(self.actions))
                            })
                            .children(self.details),
                    ),
            )
            .test_support()
            .into_any_element()
    }
}
