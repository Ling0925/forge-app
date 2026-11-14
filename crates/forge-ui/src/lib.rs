//! Forge macOS app shell: a thin gpui-kit view over `forge-state`.
//!
//! All behaviour (busy rules, idempotency ids, quit flow, labels) lives in
//! `forge-state`; this crate only renders `AppState`, turns key presses and
//! clicks into `AppState` intents, and ships the resulting commands to the
//! backend.

pub mod actions;
pub mod backend;
pub mod theme;
pub mod tokens;
mod view;

pub use view::ForgeApp;

use gpui_kit::{App, AppContext, Bounds, WindowBounds, WindowOptions, px, size};
use std::rc::Rc;

/// Registers theme, key bindings and menus. Call once after `gpui_kit::init`.
pub fn init(cx: &mut App) {
    theme::init(cx);
    actions::init(cx);
}

/// Opens the main window over `backend`.
pub fn open_main_window(
    cx: &mut App,
    backend: Rc<dyn backend::Backend>,
    events: Option<backend::EventReceiver>,
    boot_error: Option<backend::StartError>,
) {
    let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(720.), px(480.))),
        ..Default::default()
    };
    let result = gpui_kit::open_window(options, cx, move |window, cx| {
        cx.new(|cx| ForgeApp::new(backend, events, boot_error, window, cx))
    });
    if let Err(error) = result {
        eprintln!("forge-app: failed to open window: {error}");
    }
}
