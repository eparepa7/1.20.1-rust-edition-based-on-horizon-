use horizon_launcher::{backend::Settings, legacy_1201};

#[test]
fn first_run_uses_the_build_profile_without_overwriting_an_explicit_choice() {
    for text in ["", "{}", "not json", "null"] {
        assert_eq!(Settings::parse(text).dev_offline, legacy_1201::ENABLED);
    }
    let disabled = Settings::parse(r#"{"dev_offline":false,"game_args":"--world Test"}"#);
    assert!(!disabled.dev_offline);
    let saved = serde_json::to_string(&disabled).unwrap();
    let restored = Settings::parse(&saved);
    assert!(!restored.dev_offline);
    assert_eq!(restored.game_args, "--world Test");
    assert!(Settings::parse(r#"{"dev_offline":true}"#).dev_offline);
}

#[test]
fn a_fork_release_cannot_replace_itself_with_the_upstream_launcher() {
    if legacy_1201::ENABLED {
        assert!(!horizon_launcher::backend::self_update_allowed());
    }
}
