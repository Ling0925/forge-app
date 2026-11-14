//! Loads the Forge light/dark themes (§3.1 tokens mapped onto gpui-kit keys)
//! and follows the system appearance.

use gpui_kit::App;
use gpui_kit::component::{Theme, ThemeRegistry};

pub const THEME_JSON: &str = include_str!("../themes/forge.json");
pub const LIGHT_NAME: &str = "Forge Light";
pub const DARK_NAME: &str = "Forge Dark";

pub fn init(cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    if let Err(error) = registry.load_themes_from_str(THEME_JSON) {
        eprintln!("forge-app: bundled theme failed to load: {error}");
        return;
    }
    let light = registry.themes().get(LIGHT_NAME).cloned();
    let dark = registry.themes().get(DARK_NAME).cloned();
    if let (Some(light), Some(dark)) = (light, dark) {
        Theme::update(cx, |theme| {
            theme.light_theme = light;
            theme.dark_theme = dark;
        });
    }
    Theme::sync_system_appearance(None, cx);
}
