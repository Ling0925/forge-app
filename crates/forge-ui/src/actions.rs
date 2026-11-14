//! Actions, key bindings (§7) and the menu bar (§9).

use forge_state::i18n::t;
use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_kit::{App, KeyBinding, Menu, MenuItem, OsAction};

gpui_kit::actions!(
    forge,
    [
        ToggleSidebar,
        ToggleInspector,
        InspectorChanges,
        InspectorQueue,
        InspectorOutput,
        RetryLastRun,
        NewSession,
        FocusComposer,
        StopRun,
        PrevSession,
        NextSession,
        Quit,
        Steer,
        Hide,
        HideOthers,
        ShowAll,
        Minimize,
        CloseWindow,
        OpenSettings,
    ]
);

/// Key context of the root view.
pub const ROOT_CONTEXT: &str = "ForgeApp";
/// Key context around the composer textarea (⌥↩ steer lives here so it does not
/// collide with the textarea's own enter / shift-enter bindings).
pub const COMPOSER_CONTEXT: &str = "ForgeComposer";

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("ctrl-cmd-s", ToggleSidebar, None),
        KeyBinding::new("alt-cmd-i", ToggleInspector, None),
        KeyBinding::new("alt-cmd-1", InspectorChanges, None),
        KeyBinding::new("alt-cmd-2", InspectorQueue, None),
        KeyBinding::new("alt-cmd-3", InspectorOutput, None),
        KeyBinding::new("cmd-n", NewSession, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-l", FocusComposer, None),
        KeyBinding::new("cmd-.", StopRun, None),
        KeyBinding::new("cmd-r", RetryLastRun, None),
        KeyBinding::new("cmd-[", PrevSession, None),
        KeyBinding::new("cmd-]", NextSession, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("alt-enter", Steer, Some(COMPOSER_CONTEXT)),
    ]
}

pub fn menus() -> Vec<Menu> {
    vec![
        Menu::new("Forge").items([
            MenuItem::action(t("menu.app.settings"), OpenSettings),
            MenuItem::separator(),
            MenuItem::action(t("menu.app.hide"), Hide),
            MenuItem::action(t("menu.app.hide_others"), HideOthers),
            MenuItem::action(t("menu.app.show_all"), ShowAll),
            MenuItem::separator(),
            MenuItem::action(t("menu.app.quit"), Quit),
        ]),
        Menu::new(t("menu.file")).items([
            MenuItem::action(t("menu.file.new_session"), NewSession),
            MenuItem::separator(),
            MenuItem::action(t("menu.file.close_window"), CloseWindow),
        ]),
        Menu::new(t("menu.edit")).items([
            MenuItem::os_action(t("menu.edit.undo"), Undo, OsAction::Undo),
            MenuItem::os_action(t("menu.edit.redo"), Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action(t("menu.edit.cut"), Cut, OsAction::Cut),
            MenuItem::os_action(t("menu.edit.copy"), Copy, OsAction::Copy),
            MenuItem::os_action(t("menu.edit.paste"), Paste, OsAction::Paste),
            MenuItem::os_action(t("menu.edit.select_all"), SelectAll, OsAction::SelectAll),
        ]),
        Menu::new(t("menu.view")).items([
            MenuItem::action(t("menu.view.toggle_sidebar"), ToggleSidebar),
            MenuItem::action(t("menu.view.toggle_inspector"), ToggleInspector),
        ]),
        Menu::new(t("menu.session")).items([
            MenuItem::action(t("menu.session.focus_composer"), FocusComposer),
            MenuItem::action(t("menu.session.stop"), StopRun),
            MenuItem::action(t("menu.session.retry"), RetryLastRun),
            MenuItem::separator(),
            MenuItem::action(t("menu.session.prev"), PrevSession),
            MenuItem::action(t("menu.session.next"), NextSession),
        ]),
        Menu::new(t("menu.window")).items([MenuItem::action(t("menu.window.minimize"), Minimize)]),
    ]
}

pub fn init(cx: &mut App) {
    cx.bind_keys(key_bindings());
    cx.set_menus(menus());
    // App-level fallbacks when no window handles the action.
    cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
    cx.on_action(|_: &Hide, cx: &mut App| cx.hide());
    cx.on_action(|_: &HideOthers, cx: &mut App| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx: &mut App| cx.unhide_other_apps());
}
