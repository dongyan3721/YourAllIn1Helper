use super::macos::is_selection_drag;

#[test]
fn translation_drag_ignores_clicks_focus_switches_and_own_windows() {
    let pid = std::process::id() as i32 + 100;
    assert!(!is_selection_drag((10.0, 20.0, pid), (10.0, 20.0, pid)));
    assert!(!is_selection_drag((10.0, 20.0, pid), (13.0, 23.0, pid)));
    assert!(is_selection_drag((10.0, 20.0, pid), (30.0, 20.0, pid)));
    assert!(!is_selection_drag((10.0, 20.0, pid), (30.0, 20.0, pid + 1)));
    assert!(!is_selection_drag((10.0, 20.0, 0), (30.0, 20.0, 0)));
    let own = std::process::id() as i32;
    assert!(!is_selection_drag((10.0, 20.0, own), (30.0, 20.0, own)));
}
