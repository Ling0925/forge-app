//! The bundled gpui-kit theme must carry exactly the §3.1 token values.
use forge_ui::theme::{DARK_NAME, LIGHT_NAME, THEME_JSON};
use forge_ui::tokens::{DARK, GPUI_KIT_MAPPING, LIGHT, Palette};

fn hex(rgb: u32) -> String {
    format!("#{rgb:06X}")
}

fn colors(name: &str) -> serde_json::Map<String, serde_json::Value> {
    let json: serde_json::Value = serde_json::from_str(THEME_JSON).unwrap();
    json["themes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|theme| theme["name"] == name)
        .unwrap_or_else(|| panic!("theme {name} missing"))["colors"]
        .as_object()
        .unwrap()
        .clone()
}

fn check(name: &str, palette: &Palette) {
    let colors = colors(name);
    for (key, token) in GPUI_KIT_MAPPING {
        let expected = hex(palette.token(token).unwrap());
        let actual = colors
            .get(*key)
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("{name}: {key} missing"));
        assert_eq!(actual.to_uppercase(), expected, "{name}: {key} ← {token}");
    }
}

#[test]
fn light_theme_matches_tokens() {
    check(LIGHT_NAME, &LIGHT);
    assert_eq!(colors(LIGHT_NAME)["primary.background"], "#0A5BD3");
}

#[test]
fn dark_theme_matches_tokens() {
    check(DARK_NAME, &DARK);
    assert_eq!(colors(DARK_NAME)["primary.background"], "#5AA2FF");
}

#[test]
fn high_contrast_variants_raise_secondary_text_and_borders() {
    use forge_ui::theme::{DARK_HC_NAME, LIGHT_HC_NAME, high_contrast_json};
    let json: serde_json::Value = serde_json::from_str(&high_contrast_json()).unwrap();
    for (name, palette) in [(LIGHT_HC_NAME, &LIGHT), (DARK_HC_NAME, &DARK)] {
        let colors = json["themes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|theme| theme["name"] == name)
            .unwrap_or_else(|| panic!("{name} missing"))["colors"]
            .clone();
        assert_eq!(colors["muted.foreground"], hex(palette.fg), "{name}");
        assert_eq!(colors["border"], hex(palette.border_strong), "{name}");
        assert_eq!(colors["foreground"], hex(palette.fg), "{name}");
        assert_eq!(colors["primary.background"], hex(palette.accent), "{name}");
    }
}
