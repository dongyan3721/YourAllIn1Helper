use super::*;

#[test]
fn shortcut_waits_for_physical_keys_without_releasing_them() {
    let mut states = [true, true, false].into_iter();
    let mut waits = 0;
    wait_for_keyboard_release(
        || states.next().expect("must stop after release"),
        || {
            waits += 1;
            true
        },
    )
    .unwrap();
    assert_eq!(waits, 2);
}

#[test]
fn held_physical_keys_cancel_shortcut_at_deadline() {
    let mut waits = 0;
    assert!(wait_for_keyboard_release(
        || true,
        || {
            waits += 1;
            waits < 3
        },
    )
    .is_err());
    assert_eq!(waits, 3);
    wait_for_keyboard_release(|| false, || panic!("released keyboard must not wait")).unwrap();
}

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
fn forgetting_target_clears_it_so_later_paste_is_refused() {
    let mut stored = Some((0x5678, std::process::id().wrapping_add(1)));
    forget_target(&mut stored);
    assert_eq!(stored, None);
    forget_target(&mut stored);
    assert!(matches!(
        take_remembered_target(stored),
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
