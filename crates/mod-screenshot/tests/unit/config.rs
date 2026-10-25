//! 截图配置规范化、合并与未知字段保留的单元测试。
use super::*;
use serde_json::json;

#[test]
fn sanitize_repairs_invalid_values() {
    let mut s = ScreenshotSettings {
        save_directory: Some("   ".into()),
        file_name_prefix: "../../etc/pass".into(),
        default_color: "red".into(),
        default_stroke_width: f64::NAN,
        default_pin_opacity: 7.0,
        max_pins: 0,
        ..Default::default()
    };
    s.sanitize();
    assert_eq!(s.save_directory, None);
    // 路径分隔符与点被剔除，文件只能写在保存目录内。
    assert_eq!(s.file_name_prefix, "etcpass");
    assert_eq!(s.default_color, DEFAULT_ANNOTATION_COLOR);
    assert_eq!(s.default_stroke_width, 4.0);
    assert_eq!(s.default_pin_opacity, 1.0);
    assert_eq!(s.max_pins, 1);

    let mut empty_prefix = ScreenshotSettings {
        file_name_prefix: "///".into(),
        ..Default::default()
    };
    empty_prefix.sanitize();
    assert_eq!(empty_prefix.file_name_prefix, "screenshot");
}

#[test]
fn merge_keeps_untouched_and_unknown_fields() {
    let stored: ScreenshotSettings = serde_json::from_value(json!({
        "save_directory": "/tmp/shots",
        "future_option": {"keep": true}
    }))
    .unwrap();
    let merged =
        merge_settings(&stored, &json!({"copy_after_save": true, "max_pins": 999})).unwrap();
    assert_eq!(merged.save_directory.as_deref(), Some("/tmp/shots"));
    assert!(merged.copy_after_save);
    assert_eq!(merged.max_pins, MAX_PINS_LIMIT);
    let round_trip = serde_json::to_value(&merged).unwrap();
    assert_eq!(round_trip["future_option"]["keep"], true);
}

#[test]
fn merge_rejects_non_object_and_wrong_types() {
    let base = ScreenshotSettings::default();
    assert!(merge_settings(&base, &json!([1, 2])).is_err());
    assert!(merge_settings(&base, &json!({"copy_after_save": "yes"})).is_err());
}

#[test]
fn hex_color_validation() {
    assert!(is_hex_color("#00AAff"));
    assert!(!is_hex_color("#abc"));
    assert!(!is_hex_color("00aaff0"));
    assert!(!is_hex_color("#gg0000"));
}
