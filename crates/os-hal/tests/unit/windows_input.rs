use super::*;

#[test]
fn paste_target_must_be_external_and_have_matching_foreground_identity() {
    let own_process_id = std::process::id();
    let target_process_id = own_process_id.wrapping_add(1).max(1);
    let target = PasteTarget {
        hwnd: 0x1234,
        process_id: target_process_id,
    };
    assert!(is_external_target(target));
    assert!(!is_external_process(own_process_id, own_process_id));
    assert!(!is_external_process(0, own_process_id));
    assert!(process_identity_matches(
        target_process_id,
        target_process_id,
        own_process_id
    ));
    assert!(!process_identity_matches(
        target_process_id,
        target_process_id.wrapping_add(1),
        own_process_id
    ));
    assert!(!process_identity_matches(
        own_process_id,
        own_process_id,
        own_process_id
    ));
    assert!(target_is_foreground(target, 0x1234, target_process_id));
    assert!(!target_is_foreground(target, 0x5678, target_process_id));
    assert!(!target_is_foreground(
        target,
        0x1234,
        target_process_id.wrapping_add(1)
    ));
    assert!(matches!(
        take_remembered_target(None),
        Err(HalError::NotFound(_))
    ));
}

#[test]
fn remembering_own_window_clears_stale_target_and_failures_never_reuse_it() {
    let own = std::process::id();
    let own_window = PasteTarget {
        hwnd: 0x1234,
        process_id: own,
    };
    let external = PasteTarget {
        hwnd: 0x5678,
        process_id: own.wrapping_add(1),
    };
    let mut stored = Some((0x9999, own.wrapping_add(2)));

    replace_target(&mut stored, || target_for_remembering(own_window, own)).unwrap();
    assert_eq!(stored, None);

    stored = Some((0x9999, own.wrapping_add(2)));
    assert!(replace_target(&mut stored, || {
        Err(HalError::NativeCallFailed("模拟记忆失败".into()))
    })
    .is_err());
    assert_eq!(stored, None);

    replace_target(&mut stored, || target_for_remembering(external, own)).unwrap();
    assert_eq!(stored, Some((external.hwnd, external.process_id)));
}
