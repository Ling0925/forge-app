//! Loads the Forge light/dark themes (§3.1 tokens mapped onto gpui-kit keys)
//! and follows the system appearance.

use gpui_kit::component::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Window};

pub const THEME_JSON: &str = include_str!("../themes/forge.json");
pub const LIGHT_NAME: &str = "Forge Light";
pub const DARK_NAME: &str = "Forge Dark";
pub const LIGHT_HC_NAME: &str = "Forge Light HC";
pub const DARK_HC_NAME: &str = "Forge Dark HC";

/// "Increase contrast" variants (§3 / §8): secondary and tertiary text use the
/// primary foreground and hairline borders use `borderStrong`. Derived from
/// the bundled JSON by exact colour substitution so both stay in sync.
pub fn high_contrast_json() -> String {
    let mut json = THEME_JSON
        .replace(
            &format!("\"{LIGHT_NAME}\""),
            &format!("\"{LIGHT_HC_NAME}\""),
        )
        .replace(&format!("\"{DARK_NAME}\""), &format!("\"{DARK_HC_NAME}\""))
        .replace("\"is_default\": true", "\"is_default\": false");
    for palette in [&crate::tokens::LIGHT, &crate::tokens::DARK] {
        for (from, to) in [
            (palette.fg2, palette.fg),
            (palette.fg3, palette.fg),
            (palette.border, palette.border_strong),
        ] {
            json = json.replace(&format!("#{from:06X}"), &format!("#{to:06X}"));
        }
    }
    json
}

pub fn init(cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    if let Err(error) = registry.load_themes_from_str(THEME_JSON) {
        eprintln!("forge-app: bundled theme failed to load: {error}");
        return;
    }
    if let Err(error) = registry.load_themes_from_str(&high_contrast_json()) {
        eprintln!("forge-app: high-contrast theme failed to load: {error}");
    }
    let high = crate::announce::increase_contrast();
    let (light_name, dark_name) = if high {
        (LIGHT_HC_NAME, DARK_HC_NAME)
    } else {
        (LIGHT_NAME, DARK_NAME)
    };
    let light = registry.themes().get(light_name).cloned();
    let dark = registry.themes().get(dark_name).cloned();
    if let (Some(light), Some(dark)) = (light, dark) {
        Theme::update(cx, |theme| {
            theme.light_theme = light;
            theme.dark_theme = dark;
        });
    }
    sync_appearance(None, cx);
}

/// Forced appearance from `FORGE_UI_APPEARANCE=light|dark` (screenshots,
/// manual checks); `None` follows the system.
pub fn forced_mode() -> Option<ThemeMode> {
    match std::env::var("FORGE_UI_APPEARANCE").ok()?.as_str() {
        "light" => Some(ThemeMode::Light),
        "dark" => Some(ThemeMode::Dark),
        _ => None,
    }
}

/// Applies the forced appearance if set, otherwise follows the system.
pub fn sync_appearance(window: Option<&mut Window>, cx: &mut App) {
    match forced_mode() {
        Some(mode) => Theme::change(mode, window, cx),
        None => Theme::sync_system_appearance(window, cx),
    }
}
