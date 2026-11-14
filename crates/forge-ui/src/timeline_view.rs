//! Timeline row rendering (spec §6.3–§6.8) on gpui-kit `MessageScroller`,
//! `Message`/`Bubble`, `TextView` (Markdown), `Collapsible` and `Marker`.

use crate::tokens::Palette;
use crate::view::ForgeApp;
use forge_state::i18n::t;
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
use gpui_kit::component::message::{Message, MessageAlignment, MessageContent};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, Role, SharedString,
    StatefulInteractiveElement, Styled, WeakEntity, div, prelude::FluentBuilder, px, rgb,
};

/// §4: timeline content column max width.
const CONTENT_MAX: f32 = forge_state::CONTENT_MAX;

fn column(child: impl IntoElement) -> AnyElement {
    h_flex()
        .w_full()
        .justify_center()
        .px_6()
        .child(div().w_full().max_w(px(CONTENT_MAX)).child(child))
        .into_any_element()
}

pub fn render_row(row: &TimelineRowVm, app: &WeakEntity<ForgeApp>, cx: &mut App) -> AnyElement {
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
                                    .max_w(px(CONTENT_MAX * 0.8))
                                    .when(*from_queue, |b| {
                                        b.child(
                                            div()
                                                .text_xs()
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
                                .aria_label(format!("{}，{}", t("a11y.reasoning"), reasoning.label))
                                .aria_expanded(reasoning.expanded)
                                .gap_1()
                                .cursor_pointer()
                                .text_sm()
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
                                    .text_sm()
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
                    .stream_fade(*streaming),
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
                .text_sm()
                .text_color(theme.muted_foreground)
                .into_any_element()
        }
        TimelineRowVm::Error(card) => render_error_card(card, &palette, app, cx),
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
                        .text_xs()
                        .text_color(theme.muted_foreground),
                )
                .test_support()
                .into_any_element()
        }
        TimelineRowVm::Running { text, .. } => Marker::new()
            .id(key.clone())
            .alignment(MarkerAlignment::Start)
            .icon(MarkerIcon::new().child(Spinner::new().xsmall()))
            .content(MarkerContent::new().text(text.clone()))
            .text_sm()
            .text_color(theme.muted_foreground)
            .into_any_element(),
        TimelineRowVm::Notice { text, .. } => Marker::new()
            .id(key.clone())
            .alignment(MarkerAlignment::Center)
            .content(MarkerContent::new().text(text.clone()))
            .text_xs()
            .text_color(theme.muted_foreground)
            .into_any_element(),
    };
    column(element)
}

fn action_button(
    card: &ErrorCardVm,
    action: ErrorAction,
    primary: bool,
    app: &WeakEntity<ForgeApp>,
    cx: &App,
) -> Button {
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
    // Visible "重试 ⌘R"; VoiceOver reads the plain label.
    let visible = match card.primary_shortcut.filter(|_| primary && !disabled) {
        Some(shortcut) => format!("{label} {shortcut}"),
        None => label.clone(),
    };
    let weak = app.clone();
    let run = card.run_id.clone();
    let button = Button::new(id)
        .label(visible)
        .accessibility_label(label)
        .small()
        .disabled(disabled)
        .on_click(move |_, window, cx| {
            weak.update(cx, |this, cx| this.error_action(&run, action, window, cx))
                .ok();
        });
    // §3.1: the error card's primary "重试" uses dangerBtn / onDanger.
    let button = match (primary, action) {
        (true, ErrorAction::Retry) => button.danger(),
        (true, _) => button.primary(),
        (false, _) => button.outline(),
    };
    crate::style::disabled_readable(button, disabled, cx)
}

fn render_error_card(
    card: &ErrorCardVm,
    palette: &Palette,
    app: &WeakEntity<ForgeApp>,
    cx: &mut App,
) -> AnyElement {
    let icon = match card.kind {
        ErrorCardKind::Interrupted | ErrorCardKind::RateLimitedWait => IconName::TriangleAlert,
        _ => IconName::CircleX,
    };
    let fg = rgb(palette.fg);
    v_flex()
        .id(SharedString::from(format!("error-{}", card.run_id.0)))
        .role(Role::Group)
        .aria_label(card.a11y_label.clone())
        .aria_description(card.body.clone())
        .p_4()
        .gap_2()
        .rounded(px(8.))
        .bg(rgb(palette.danger_bg))
        .border_1()
        .border_color(rgb(palette.danger))
        .text_color(fg)
        .child(
            h_flex()
                .gap_2()
                .items_start()
                .child(Icon::new(icon).text_color(rgb(palette.danger)))
                .child(
                    v_flex()
                        .gap_0p5()
                        .child(
                            div()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .child(card.title.clone()),
                        )
                        .child(div().text_sm().child(card.body.clone())),
                ),
        )
        .child(
            h_flex()
                .pl_6()
                .gap_2()
                .child(action_button(card, card.primary, true, app, cx))
                .child(action_button(card, card.secondary, false, app, cx)),
        )
        .when(card.details_open, |el| {
            el.child(
                v_flex()
                    .id(SharedString::from(format!(
                        "error-details-{}",
                        card.run_id.0
                    )))
                    .ml_6()
                    .p_2()
                    .rounded(px(6.))
                    .bg(rgb(palette.surface))
                    .font_family("SF Mono")
                    .text_xs()
                    .children(card.details.iter().map(|line| div().child(line.clone())))
                    .test_support(),
            )
        })
        .test_support()
        .into_any_element()
}
