use super::*;

#[test]
fn config_normalizes_out_of_range_values() {
    let config = AppUsageConfig {
        idle_threshold_secs: 0,
        flush_interval_secs: 1,
        keep_window_titles: false,
        audio_afk_exception: false,
    }
    .normalized();
    assert_eq!(config.idle_threshold_secs, MIN_IDLE_SECS);
    assert_eq!(config.flush_interval_secs, MIN_FLUSH_SECS);
    assert!(!config.keep_window_titles);
    assert!(!config.audio_afk_exception);
    assert!(config.validate().is_ok());
}

#[test]
fn legacy_config_without_audio_flag_defaults_to_enabled() {
    let config: AppUsageConfig = serde_json::from_str(
        r#"{"idle_threshold_secs": 120, "flush_interval_secs": 30, "keep_window_titles": true}"#,
    )
    .unwrap();
    assert!(config.audio_afk_exception);
}

#[test]
fn category_accepts_only_known_values() {
    assert_eq!(normalize_category("Work"), Some("work".to_string()));
    assert_eq!(normalize_category("unknown"), None);
    assert_eq!(normalize_category("  "), None);
}
