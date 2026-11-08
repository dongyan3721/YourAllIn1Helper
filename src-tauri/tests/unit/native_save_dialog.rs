//! 系统保存对话框的参数、取消与路径授权纯逻辑测试。
use super::*;

#[test]
fn save_path_registry_is_one_shot_and_matches_kind() {
    let registry = SavePathRegistry::default();
    let path = PathBuf::from("/tmp/媒体 文件/截图.png");
    registry.register(path.clone(), SaveKind::Png);
    // 未登记的路径被拒绝（模拟 HTTP/WebView 直接指定任意路径）。
    assert!(registry
        .consume(Path::new("/tmp/other.png"), SaveKind::Png)
        .is_err());
    // 格式不匹配被拒绝。
    assert!(registry.consume(&path, SaveKind::Gif).is_err());
    // 首次消费成功，重放（同一路径二次覆盖写）被拒绝。
    assert!(registry.consume(&path, SaveKind::Png).is_ok());
    assert!(registry.consume(&path, SaveKind::Png).is_err());
}

#[test]
fn save_path_registry_evicts_oldest_beyond_capacity() {
    let registry = SavePathRegistry::default();
    for index in 0..(MAX_RECENT_SELECTIONS + 1) {
        registry.register(PathBuf::from(format!("p{}.png", index)), SaveKind::Png);
    }
    assert!(registry
        .consume(Path::new("p0.png"), SaveKind::Png)
        .is_err());
    assert!(registry.consume(Path::new("p8.png"), SaveKind::Png).is_ok());
}

#[test]
fn save_path_registry_rejects_cancel_that_never_registered() {
    let registry = SavePathRegistry::default();
    // 取消不登记：后续伪造同一路径的写入必须被拒绝。
    assert!(registry
        .consume(Path::new("/tmp/未选择.png"), SaveKind::Png)
        .is_err());
}

#[test]
fn sanitize_file_name_keeps_chinese_spaces_and_strips_separators() {
    assert_eq!(
        sanitize_file_name("截图 文件 01", SaveKind::Png),
        "截图 文件 01.png"
    );
    // 目录穿越只取叶子名，非法字符替换为下划线。
    assert_eq!(
        sanitize_file_name("..\\..\\evil name.png", SaveKind::Png),
        "evil name.png"
    );
    assert_eq!(sanitize_file_name("a/b\\c:*.gif", SaveKind::Gif), "c__.gif");
    // 空 / 纯空白退回安全缺省名。
    assert_eq!(sanitize_file_name("", SaveKind::Png), "screenshot.png");
    assert_eq!(sanitize_file_name("   ", SaveKind::Gif), "screenshot.gif");
    // 显式扩展名保持原样。
    assert_eq!(sanitize_file_name("动图.PNG", SaveKind::Gif), "动图.PNG");
}

#[test]
fn parse_spec_validates_kind_and_fills_labels() {
    let spec = parse_spec(&json!({"kind": "GIF", "default_name": "x/y"})).unwrap();
    assert_eq!(spec.kind, SaveKind::Gif);
    assert_eq!(spec.default_file_name, "y.gif");
    assert_eq!(spec.filter_label, "GIF animation");
    assert_eq!(spec.title, "保存截图");
    assert!(parse_spec(&json!({"kind": "jpg"})).is_err());
    assert!(parse_spec(&json!({})).is_err());
}

#[test]
fn module_disable_is_rejected_before_any_dialog_work() {
    let dir = tempfile::tempdir().unwrap();
    let host = DesktopHost::bootstrap(dir.path(), true).unwrap();
    host.kernel()
        .set_module_enabled("screenshot", false)
        .unwrap();
    assert!(ensure_screenshot_enabled(&host).is_err());
    host.kernel()
        .set_module_enabled("screenshot", true)
        .unwrap();
    assert!(ensure_screenshot_enabled(&host).is_ok());
}
