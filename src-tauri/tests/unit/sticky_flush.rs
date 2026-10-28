use super::*;

#[test]
fn page_confirmation_resolves_waiter_with_success() {
    let coordinator = Arc::new(StickyFlushCoordinator::new());
    let (request_id, signal) = coordinator.begin("note-a");
    let waiter = {
        let coordinator = coordinator.clone();
        let signal = signal.clone();
        std::thread::spawn(move || {
            coordinator.wait("note-a", request_id, signal, Duration::from_secs(2))
        })
    };
    assert!(coordinator.complete("note-a", request_id, Ok(())));
    assert_eq!(waiter.join().unwrap(), CloseFlushOutcome::Saved);
    assert!(!coordinator.complete("note-a", request_id, Ok(())));
}

#[test]
fn page_failure_is_reported_and_keeps_the_window_decision_with_the_caller() {
    let coordinator = StickyFlushCoordinator::new();
    let (request_id, signal) = coordinator.begin("note-b");
    assert!(coordinator.complete("note-b", request_id, Err("disk full".to_string())));
    assert_eq!(
        coordinator.wait("note-b", request_id, signal, Duration::from_secs(1)),
        CloseFlushOutcome::Failed("disk full".to_string())
    );
}

#[test]
fn missing_page_response_times_out_without_polling_the_clock_forever() {
    let coordinator = StickyFlushCoordinator::new();
    let (request_id, signal) = coordinator.begin("note-c");
    let started = Instant::now();
    assert_eq!(
        coordinator.wait("note-c", request_id, signal, Duration::from_millis(30)),
        CloseFlushOutcome::TimedOut
    );
    assert!(started.elapsed() >= Duration::from_millis(25));
    assert!(started.elapsed() < Duration::from_secs(1));
    // The timed-out waiter is removed so a late page response cannot leak state.
    assert!(!coordinator.complete("note-c", request_id, Ok(())));
}

#[test]
fn late_confirmation_after_timeout_cannot_release_a_newer_request() {
    let coordinator = StickyFlushCoordinator::new();

    let (old_id, old_signal) = coordinator.begin("note-d");
    assert_eq!(
        coordinator.wait("note-d", old_id, old_signal, Duration::from_millis(10)),
        CloseFlushOutcome::TimedOut
    );

    let (new_id, new_signal) = coordinator.begin("note-d");
    assert_ne!(old_id, new_id, "retries must have unique request ids");
    assert!(
        !coordinator.complete("note-d", old_id, Ok(())),
        "the late ack for the timed-out request must be rejected"
    );
    assert!(
        !coordinator.complete("note-d", old_id, Err("stale".into())),
        "a stale failure result must not remove the new waiter"
    );

    assert!(coordinator.complete("note-d", new_id, Ok(())));
    assert_eq!(
        coordinator.wait("note-d", new_id, new_signal, Duration::from_secs(1)),
        CloseFlushOutcome::Saved
    );
}

#[test]
fn unknown_or_duplicate_acknowledgements_are_rejected() {
    let coordinator = StickyFlushCoordinator::new();
    assert!(!coordinator.complete("missing", 1, Ok(())));
    let (request_id, signal) = coordinator.begin("note-e");
    assert!(coordinator.complete("note-e", request_id, Ok(())));
    assert!(!coordinator.complete("note-e", request_id, Ok(())));
    drop(signal);
}
