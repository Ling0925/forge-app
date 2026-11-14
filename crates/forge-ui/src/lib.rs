//! Forge macOS app shell: a thin gpui-kit view over `forge-state`.
//!
//! All behaviour (busy rules, idempotency ids, quit flow, labels) lives in
//! `forge-state`; this crate only renders `AppState`, turns key presses and
//! clicks into `AppState` intents, and ships the resulting commands to the
//! backend.

pub mod actions;
pub mod announce;
pub mod backend;
pub mod demo;
pub mod theme;
mod timeline_view;
pub mod tokens;
mod view;

pub use view::ForgeApp;

use gpui_kit::{
    App, AppContext, Bounds, TitlebarOptions, Window, WindowBounds, WindowOptions, px, size,
};
use std::rc::Rc;

/// Registers theme, key bindings and menus. Call once after `gpui_kit::init`.
pub fn init(cx: &mut App) {
    theme::init(cx);
    actions::init(cx);
}

/// Main window options: §4 default 1280×800, minimum 960×600.
pub fn window_options(cx: &mut App) -> WindowOptions {
    let bounds = Bounds::centered(
        None,
        size(
            px(forge_state::DEFAULT_WINDOW_WIDTH),
            px(forge_state::DEFAULT_WINDOW_HEIGHT),
        ),
        cx,
    );
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(
            px(forge_state::MIN_WINDOW_WIDTH),
            px(forge_state::MIN_WINDOW_HEIGHT),
        )),
        titlebar: Some(TitlebarOptions {
            title: Some(forge_state::i18n::t("app.title").into()),
            ..Default::default()
        }),
        app_id: Some("forge-app".into()),
        ..Default::default()
    }
}

/// Opens the main window over `backend`.
pub fn open_main_window(
    cx: &mut App,
    backend: Rc<dyn backend::Backend>,
    events: Option<backend::EventReceiver>,
    boot_error: Option<backend::StartError>,
) {
    open_with(cx, move |window, cx| {
        ForgeApp::new(backend, events, boot_error, window, cx)
    });
}

/// Opens the main window on a fixture scene (`FORGE_UI_DEMO`).
pub fn open_demo_window(cx: &mut App, scene: demo::Scene) {
    open_with(cx, move |window, cx| {
        let mut app = ForgeApp::new(Rc::new(demo::DemoBackend), None, None, window, cx);
        let now = demo::populate(&mut app.state, scene);
        app.set_clock_base(now);
        app.after_change(window, cx);
        app
    });
}

fn open_with(
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut gpui_kit::Context<ForgeApp>) -> ForgeApp + 'static,
) {
    let options = window_options(cx);
    let result = gpui_kit::open_window(options, cx, move |window, cx| {
        cx.new(|cx| build(window, cx))
    });
    if let Err(error) = result {
        eprintln!("forge-app: failed to open window: {error}");
    }
}
