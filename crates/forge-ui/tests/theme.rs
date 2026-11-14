//! The bundled gpui-kit theme must carry exactly the §3.1 token values.
use forge_ui::theme::{DARK_NAME, LIGHT_NAME, THEME_JSON};
use forge_ui::tokens::{DARK, GPUI_KIT_MAPPING, LIGHT, Palette, SCRIM_DARK, SCRIM_LIGHT};

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

/// 2026-11-14 D1: "停止 ⌘." uses `danger` for text (≥ 4.5:1) and its 1pt
/// border (≥ 3:1) on the composer surface and window, both themes; also on
/// its `dangerBg` hover fill. `dangerBtn` text on dark was 2.55:1.
#[test]
fn stop_button_danger_contrast() {
    use forge_ui::tokens::contrast;
    for p in [&LIGHT, &DARK] {
        for bg in [p.surface, p.window, p.danger_bg] {
            let c = contrast(p.danger, bg);
            assert!(c >= 4.5, "danger on {bg:06X}: {c:.2}");
        }
    }
    assert!(
        contrast(DARK.danger_btn, DARK.surface) < 4.5,
        "why dangerBtn is not used"
    );
}

#[test]
fn overlay_key_carries_the_scrim_token_per_theme() {
    // Settings scrim: black 30 % light, black 50 % dark.
    assert_eq!(colors(LIGHT_NAME)["overlay"], format!("#{SCRIM_LIGHT:08X}"));
    assert_eq!(colors(DARK_NAME)["overlay"], format!("#{SCRIM_DARK:08X}"));
    assert_eq!(colors(LIGHT_NAME)["overlay"], "#0000004D");
    assert_eq!(colors(DARK_NAME)["overlay"], "#00000080");
    assert_eq!(forge_ui::tokens::scrim(false), SCRIM_LIGHT);
    assert_eq!(forge_ui::tokens::scrim(true), SCRIM_DARK);
}
