use super::*;

#[test]
fn config_normalizes_out_of_range_values() {
    let config = AppUsageConfig {
        idle_threshold_secs: 0,
        flush_interval_secs: 1,
        keep_window_titles: false,
    }
    .normalized();
    assert_eq!(config.idle_threshold_secs, MIN_IDLE_SECS);
    assert_eq!(config.flush_interval_secs, MIN_FLUSH_SECS);
    assert!(!config.keep_window_titles);
    assert!(config.validate().is_ok());
}

#[test]
fn category_accepts_only_known_values() {
    assert_eq!(normalize_category("Work"), Some("work".to_string()));
    assert_eq!(normalize_category("unknown"), None);
    assert_eq!(normalize_category("  "), None);
}
