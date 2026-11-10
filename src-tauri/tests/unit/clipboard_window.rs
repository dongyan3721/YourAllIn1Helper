use super::*;

#[test]
fn already_foreground_popup_is_accepted_without_waiting() {
    assert!(wait_until_foreground(
        || true,
        || panic!("must not wait when the popup already owns the foreground")
    ));
}

#[test]
fn popup_that_becomes_foreground_after_a_few_polls_is_accepted() {
    let mut checks = 0;
    let mut waits = 0;
    let acquired = wait_until_foreground(
        || {
            checks += 1;
            checks == 3
        },
        || {
            waits += 1;
            true
        },
    );
    assert!(acquired);
    assert_eq!(waits, 2);
}

#[test]
fn denied_foreground_stops_at_the_deadline_instead_of_retrying_forever() {
    let mut checks = 0;
    let mut waits = 0;
    let acquired = wait_until_foreground(
        || {
            checks += 1;
            false
        },
        || {
            waits += 1;
            waits < 4
        },
    );
    assert!(!acquired);
    assert_eq!(waits, 4);
    // One check before every wait plus the last one that preceded the refused wait.
    assert_eq!(checks, 4);
}

#[test]
fn only_the_popup_handle_counts_as_foreground() {
    assert!(popup_is_foreground(0x1234, 0x1234));
    assert!(!popup_is_foreground(0x1234, 0x5678));
    assert!(!popup_is_foreground(0x1234, 0));
    assert!(!popup_is_foreground(0, 0));
}

#[test]
fn denial_message_names_the_window_that_kept_the_foreground() {
    let message = foreground_denied_message(0x1234, 0x5678, 4242);
    assert!(message.contains("4242"));
    assert!(message.contains("0x5678"));
    assert!(message.contains("0x1234"));
    let none = foreground_denied_message(0x1234, 0, 0);
    assert!(none.contains("没有前台窗口"));
}

#[test]
fn timing_constants_keep_the_wait_short_and_bounded() {
    assert!(FOREGROUND_POLL < FOREGROUND_TIMEOUT);
    assert!(FOREGROUND_TIMEOUT <= Duration::from_millis(500));
}
