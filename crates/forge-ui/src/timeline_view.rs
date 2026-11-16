//! Timeline row rendering (spec §6.3–§6.8) on gpui-kit `MessageScroller`,
//! `Message`/`Bubble`, `TextView` (Markdown), `Collapsible` and `Marker`.
//!
//! Visual v2.1: user bubble `bubble`/radius-lg at 70% width, message 14/22,
//! the run status line is a glowing accent dot + footnote, and the error card
//! is `dangerSoft` with a 3pt `danger` bar, a Semibold danger header
//! ("运行失败：<类型>"), a one-line reason and 重试 as the (single) primary
//! button — details stay behind "查看日志".

use crate::actions::CloseSettings;
use crate::focus_help::{self, FocusHelp};
use crate::tokens::{self, Palette};
use crate::view::ForgeApp;
use forge_state::i18n::{t, tf};
use forge_state::{
    DividerIcon, ErrorAction, ErrorCardKind, ErrorCardVm, RetryButtonState, TimelineRowVm,
};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::bubble::{Bubble, BubbleContent, BubbleVariant};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::collapsible::Collapsible;
use gpui_kit::component::marker::{
    Marker, MarkerAlignment, MarkerContent, MarkerIcon, MarkerVariant,
};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::message::{Message, MessageAlignment, MessageContent};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, ClickEvent, InteractiveElement, IntoElement, ParentElement, Role,
    SharedString, StatefulInteractiveElement, Styled, WeakEntity, Window, div,
    prelude::FluentBuilder, px, rgb,
};
use std::rc::Rc;

/// §4: timeline content column max width.
const CONTENT_MAX: f32 = forge_state::CONTENT_MAX;

fn column(child: impl IntoElement) -> AnyElement {
    h_flex()
        .w_full()
        .justify_center()
        .px_8()
        .child(div().w_full().max_w(px(CONTENT_MAX)).child(child))
        .into_any_element()
}

pub fn render_row(
    row: &TimelineRowVm,
    app: &WeakEntity<ForgeApp>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let theme = cx.theme().clone();
    let palette = *Palette::for_dark(theme.is_dark());
    let key = SharedString::from(row.key().to_owned());
    let element = match row {
        TimelineRowVm::User {
            text,
            from_queue,
            a11y_label,
            ..
        } => div()
            .id(key.clone())
            .role(Role::ListItem)
            .aria_label(a11y_label.clone())
            .child(
                Message::new().alignment(MessageAlignment::End).content(
                    MessageContent::new().bubble(
                        Bubble::new()
                            .alignment(MessageAlignment::End)
                            .with_variant(BubbleVariant::Secondary)
                            .content(
                                BubbleContent::new()
                                    .max_w(px(CONTENT_MAX * 0.7))
                                    .rounded(px(tokens::component::bubble::RADIUS))
                                    .bg(rgb(tokens::component::bubble::bg(&palette)))
                                    .text_color(rgb(palette.fg))
                                    .px(px(tokens::component::bubble::PAD_X))
                                    .py(px(tokens::component::bubble::PAD_Y))
                                    .text_size(px(tokens::MESSAGE.size))
                                    .line_height(px(tokens::MESSAGE.line_height))
                                    .when(*from_queue, |b| {
                                        b.child(
                                            div()
                                                .text_size(px(tokens::FOOTNOTE.size))
                                                .text_color(theme.muted_foreground)
                                                .child(t("timeline.from_queue")),
                                        )
                                    })
                                    .child(text.clone()),
                            ),
                    ),
                ),
            )
            .test_support()
            .into_any_element(),
        TimelineRowVm::Assistant {
            run_id,
            text,
            streaming,
            reasoning,
            a11y_label,
            ..
        } => {
            let mut body = v_flex().gap_2();
            if let Some(reasoning) = reasoning {
                let weak = app.clone();
                let run = run_id.clone();
                let chevron = if reasoning.expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                };
                body = body.child(
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .id(SharedString::from(format!("reasoning-{}", run_id.0)))
                                .role(Role::Button)
                                .aria_label(tf(
                                    "a11y.reasoning.toggle",
                                    &[("name", t("a11y.reasoning")), ("label", &reasoning.label)],
                                ))
                                .aria_expanded(reasoning.expanded)
                                .gap_1()
                                .cursor_pointer()
                                .text_size(px(tokens::FOOTNOTE.size))
                                .line_height(px(tokens::FOOTNOTE.line_height))
                                .text_color(theme.muted_foreground)
                                .child(Icon::new(chevron).xsmall())
                                .child(reasoning.label.clone())
                                .on_click(move |_, window, cx| {
                                    weak.update(cx, |this, cx| {
                                        this.toggle_reasoning(&run, window, cx)
                                    })
                                    .ok();
                                })
                                .test_support(),
                        )
                        .child(
                            Collapsible::new().open(reasoning.expanded).content(
                                div()
                                    .pl_4()
                                    .ml_1()
                                    .border_l_2()
                                    .border_color(theme.border)
                                    .text_size(px(tokens::FOOTNOTE.size))
                                    .text_color(theme.muted_foreground)
                                    .child(reasoning.text.clone()),
                            ),
                        ),
                );
            }
            if !text.is_empty() {
                body = body.child(
                    TextView::markdown(
                        SharedString::from(format!("md-{}", run_id.0)),
                        text.clone(),
                    )
                    .selectable(true)
                    .stream_fade(*streaming)
                    // v2 `message`: 14pt, 22pt leading.
                    .text_size(px(tokens::MESSAGE.size))
                    .line_height(px(tokens::MESSAGE.line_height)),
                );
            }
            div()
                .id(key.clone())
                .role(Role::ListItem)
                .aria_label(a11y_label.clone())
                .child(Message::new().content(MessageContent::new().child(body)))
                .test_support()
                .into_any_element()
        }
        TimelineRowVm::Retry { run_id, marker, .. } => {
            let weak = app.clone();
            Marker::new()
                .id(key.clone())
                .alignment(MarkerAlignment::Center)
                .icon(
                    MarkerIcon::new()
                        .child(Icon::new(IconName::RotateCw).text_color(rgb(palette.warning))),
                )
                .content(MarkerContent::new().text(marker.text.clone()))
                .child(
                    Button::new(SharedString::from(format!("retry-stop-{}", run_id.0)))
                        .label(t("retry.stop"))
                        .ghost()
                        .xsmall()
                        .on_click(move |_, window, cx| {
                            weak.update(cx, |this, cx| this.stop(window, cx)).ok();
                        }),
                )
                .text_size(px(tokens::FOOTNOTE.size))
                .text_color(theme.muted_foreground)
                .into_any_element()
        }
        TimelineRowVm::Error(card) => render_error_card(card, &palette, app, window, cx),
        TimelineRowVm::Divider {
            icon,
            text,
            a11y_label,
            ..
        } => {
            let (icon, color) = match icon {
                DividerIcon::Success => (IconName::CircleCheck, palette.success),
                DividerIcon::Failed => (IconName::CircleX, palette.danger),
                DividerIcon::Stopped => (IconName::Square, palette.fg2),
                DividerIcon::Interrupted => (IconName::TriangleAlert, palette.warning),
            };
            div()
                .id(key.clone())
                .role(Role::Group)
                .aria_label(a11y_label.clone())
                .child(
                    Marker::new()
                        .with_variant(MarkerVariant::Separator)
                        .alignment(MarkerAlignment::Center)
                        .icon(
                            MarkerIcon::new()
                                .child(Icon::new(icon).xsmall().text_color(rgb(color))),
                        )
                        .content(MarkerContent::new().text(text.clone()))
                        .text_size(px(tokens::FOOTNOTE.size))
                        .line_height(px(tokens::FOOTNOTE.line_height))
                        .text_color(rgb(palette.fg3)),
                )
                .test_support()
                .into_any_element()
        }
        // v2.1 run status: glowing accent dot + "运行中 · 0:10" (footnote fg2).
        TimelineRowVm::Running { text, .. } => Marker::new()
            .id(key.clone())
            .alignment(MarkerAlignment::Start)
            .icon(
                MarkerIcon::new().child(
                    div()
                        .size(px(tokens::SPACE_2))
                        .rounded_full()
                        .bg(rgb(palette.accent))
                        .shadow(vec![
                            gpui_kit::BoxShadow::new(
                                px(0.),
                                px(0.),
                                rgb(palette.accent).opacity(0.18).into(),
                            )
                            .spread_radius(px(3.)),
                        ]),
                ),
            )
            .content(MarkerContent::new().text(text.clone()))
            .text_size(px(tokens::FOOTNOTE.size))
            .line_height(px(tokens::FOOTNOTE.line_height))
            .text_color(rgb(palette.fg2))
            .into_any_element(),
        TimelineRowVm::Notice { text, .. } => Marker::new()
            .id(key.clone())
            .alignment(MarkerAlignment::Center)
            .content(MarkerContent::new().text(text.clone()))
            .text_size(px(tokens::FOOTNOTE.size))
            .line_height(px(tokens::FOOTNOTE.line_height))
            .text_color(rgb(palette.fg3))
            .into_any_element(),
    };
    column(element)
}

/// v2.1 action button: the visible label is the action name only — the
/// shortcut (⌘R / ⌘N, latest card only) sits on the tooltip (hover), the
/// §0 focus chip (keyboard) and the AccessKit description (screen readers).
/// 重试 is the one primary button. Returns the element plus the
/// `FocusHelp` scope so the card can dismiss the chip on Escape.
fn action_button(
    card: &ErrorCardVm,
    action: ErrorAction,
    primary: bool,
    app: &WeakEntity<ForgeApp>,
    palette: &Palette,
    window: &mut Window,
    cx: &mut App,
) -> (AnyElement, FocusHelp) {
    let id = SharedString::from(format!(
        "error-{}-{}",
        card.run_id.0,
        if primary { "primary" } else { "secondary" }
    ));
    let (label, disabled) = match action {
        ErrorAction::Retry => (
            card.retry_label.clone(),
            matches!(card.retry, RetryButtonState::Waiting { .. }),
        ),
        ErrorAction::Details if card.details_open => (t("error.details.hide").to_owned(), false),
        _ => (t(action.label_key()).to_owned(), false),
    };
    let shortcut = card.primary_shortcut.filter(|_| primary && !disabled);
    let tip: Option<SharedString> = shortcut.map(|shortcut| {
        tf(
            "error.action.with_shortcut",
            &[("label", t(action.label_key())), ("shortcut", shortcut)],
        )
        .into()
    });
    let help = focus_help::state(window, cx, format!("{id}-help"));
    let show = help.showing(tip.is_some() && help.focused(window, cx));
    let weak = app.clone();
    let run = card.run_id.clone();
    let button = Button::new(id.clone())
        .label(label.clone())
        .accessibility_label(label)
        .small()
        // §0: the old 28pt card buttons are 32pt now.
        .h(px(tokens::component::button::HEIGHT))
        .disabled(disabled)
        .when_some(tip.clone(), |button, tip| button.tooltip(tip))
        .on_click(move |_, window, cx| {
            weak.update(cx, |this, cx| this.error_action(&run, action, window, cx))
                .ok();
        });
    // v2.1 §4: one primary button per card ("重试" is no longer dangerBtn);
    // secondaries are neutral outlines with the ≥3:1 `borderStrong` stroke.
    let button = match primary {
        true => button.primary(),
        false => button
            .outline()
            .border_color(rgb(cx_palette_border_strong(cx))),
    };
    let button = crate::style::disabled_readable(button, disabled, cx);
    let element = match tip {
        Some(tip) => focus_help::anchor(
            &help,
            button,
            focus_help::bubble(&help, format!("{id}-tip"), tip, show, palette, cx),
        )
        .into_any_element(),
        None => button.into_any_element(),
    };
    (element, help)
}

/// `borderStrong` from the app's palette for outline buttons inside cards.
fn cx_palette_border_strong(cx: &App) -> u32 {
    Palette::for_dark(cx.theme().is_dark()).border_strong
}

/// v2.1 error card (§4 组件修正 1–2): `dangerSoft` fill, 3pt `danger` left
/// bar, header is the ✕ icon + Semibold "运行失败：<错误类型>" in `danger`
/// (no status pill — the icon and bar carry the state), one-line reason in
/// `fg`, `meta` in `fg3`, actions = primary 重试 + outline "查看日志" + the
/// ⋯ overflow (复制错误信息). Full detail lines stay in the toggle. §0:
/// every tooltip-only fact (the ⋯ label, the primary shortcut) also shows
/// as a focus chip; Escape hides it and keeps bubbling.
fn render_error_card(
    card: &ErrorCardVm,
    palette: &Palette,
    app: &WeakEntity<ForgeApp>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    use tokens::component::card as ct;
    let id = format!("error-{}", card.run_id.0);
    let icon = match card.kind {
        ErrorCardKind::Interrupted | ErrorCardKind::RateLimitedWait => IconName::TriangleAlert,
        _ => IconName::CircleX,
    };
    let details = card.details_open.then(|| {
        v_flex()
            .id(SharedString::from(format!(
                "error-details-{}",
                card.run_id.0
            )))
            .mt_1()
            .ml(px(tokens::SPACE_6))
            .p_2()
            .rounded(px(tokens::RADIUS_SM))
            .bg(rgb(palette.surface))
            .font_family("SF Mono")
            .text_size(px(tokens::FOOTNOTE.size))
            .children(card.details.iter().map(|line| div().child(line.clone())))
            .test_support()
            .into_any_element()
    });
    let copy_text = card.details.join("\n");
    type ClickFn = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
    let copy: ClickFn = Rc::new(move |_, _, cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(copy_text.clone()));
    });
    let more_id = SharedString::from(format!("{id}-more"));
    let more_help = focus_help::state(window, cx, format!("{more_id}-help"));
    let more_show = more_help.showing(more_help.focused(window, cx));
    let (primary_button, primary_help) =
        action_button(card, card.primary, true, app, palette, window, cx);
    let (secondary_button, _) =
        action_button(card, card.secondary, false, app, palette, window, cx);
    v_flex()
        .id(SharedString::from(id.clone()))
        .role(Role::Group)
        .aria_label(card.a11y_label.clone())
        .aria_description(card.body.clone())
        .on_action({
            // Escape here resolves to `CloseSettings` (root context) —
            // hide the card's help chips, then keep the event moving so
            // the settings/root Escape behaviours still run.
            let primary_help = primary_help.clone();
            let more_help = more_help.clone();
            move |_: &CloseSettings, window, cx| {
                primary_help.dismiss(window, cx);
                more_help.dismiss(window, cx);
            }
        })
        .relative()
        .overflow_hidden()
        .p(px(ct::PADDING))
        .rounded(px(ct::RADIUS))
        .bg(rgb(ct::error::bg(palette)))
        .text_color(rgb(palette.fg))
        .child(
            // 3pt danger accent bar (a stroke, not spacing).
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(ct::error::BAR))
                .bg(rgb(ct::error::bar(palette))),
        )
        .child(
            h_flex()
                .id(SharedString::from(format!("{id}-status")))
                .gap_2()
                .items_center()
                .text_size(px(tokens::BODY.size))
                .line_height(px(tokens::BODY.line_height))
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(rgb(ct::error::title_fg(palette)))
                .child(
                    Icon::new(icon)
                        .xsmall()
                        .text_color(rgb(ct::error::title_fg(palette))),
                )
                .child(card.a11y_label.clone())
                .child(div().flex_1())
                .child(focus_help::anchor(
                    &more_help,
                    Button::new(more_id.clone())
                        .icon(IconName::Ellipsis)
                        .ghost()
                        .xsmall()
                        .accessibility_label(t("card.more"))
                        .tooltip(t("card.more"))
                        .dropdown_menu(move |mut menu, _, _| {
                            let copy = copy.clone();
                            menu = menu.item(
                                PopupMenuItem::new(t("error.copy_details"))
                                    .on_click(move |e, w, cx| copy(e, w, cx)),
                            );
                            menu
                        }),
                    focus_help::bubble(
                        &more_help,
                        format!("{more_id}-tip"),
                        t("card.more"),
                        more_show,
                        palette,
                        cx,
                    ),
                ))
                .test_support(),
        )
        .child(
            div()
                .mt_2()
                .ml(px(tokens::SPACE_6))
                .text_size(px(tokens::BODY.size))
                .line_height(px(tokens::BODY.line_height))
                .text_color(rgb(ct::error::body_fg(palette)))
                .child(card.body.clone()),
        )
        // v2.1 keeps the card to header + one-line reason; the provider/time
        // `meta` stays on the VM but off the visual surface.
        .child(
            h_flex()
                .mt_3()
                .ml(px(tokens::SPACE_6))
                .gap_2()
                .child(primary_button)
                .child(secondary_button),
        )
        .children(details)
        .test_support()
        .into_any_element()
}
