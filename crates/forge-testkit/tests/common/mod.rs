use forge_protocol::*;

pub fn settings(provider: &str) -> Settings {
    Settings {
        providers: vec![ProviderSettings {
            id: provider.into(),
            endpoint: None,
            keychain_reference: None,
            max_output_tokens: Some(128),
        }],
        model: ModelSelection {
            provider: provider.into(),
            model: "fixture-model".into(),
        },
        permission_mode: PermissionMode::Ask,
        ui_prefs: Default::default(),
    }
}
