use super::*;

struct UnavailableState {
    watcher: FakeWatcher,
    fail_lock: bool,
}

impl ActiveAppWatcher for UnavailableState {
    fn current_active_app(&self) -> HalResult<ActiveAppSnapshot> {
        self.watcher.current_active_app()
    }
    fn system_idle_seconds(&self) -> HalResult<u64> {
        if self.fail_lock {
            Ok(0)
        } else {
            Err(os_hal::HalError::NativeCallFailed(
                "idle unavailable".into(),
            ))
        }
    }
    fn is_screen_locked(&self) -> HalResult<bool> {
        if self.fail_lock {
            Err(os_hal::HalError::NativeCallFailed(
                "lock unavailable".into(),
            ))
        } else {
            Ok(false)
        }
    }
}

#[test]
fn unavailable_idle_or_lock_state_never_counts_as_active_usage() {
    for fail_lock in [false, true] {
        let dir = temp_dir("unavailable-state");
        let ctx = context(dir.path(), true);
        let clock = Arc::new(FakeClock::at(1_790_000_000_000));
        let module = AppUsageModule::with_clock(
            Arc::new(UnavailableState {
                watcher: FakeWatcher::new("editor", "Editor"),
                fail_lock,
            }),
            clock.clone(),
        );
        module.sample_now(&ctx).unwrap();
        run_seconds(&module, &ctx, &clock, 5);
        module.tracker.finish(&ctx).unwrap();
        assert_eq!(total_seconds(&ctx), 0);
        let caps = module.capabilities();
        assert_eq!(caps.screen_lock_detection, !fail_lock);
        assert_eq!(caps.idle_detection, fail_lock);
    }
}

fn sql(ctx: &ModuleContext, sql: &str) {
    ctx.db
        .with_conn(|conn| {
            conn.execute_batch(sql)?;
            Ok(())
        })
        .unwrap();
}

#[test]
fn disabling_titles_scrubs_the_active_session_and_pending_metadata() {
    let dir = temp_dir("title-toggle");
    let ctx = context(dir.path(), true);
    let clock = Arc::new(FakeClock::at(1_790_000_000_000));
    let module = AppUsageModule::with_clock(
        Arc::new(FakeWatcher::new("editor", "Secret title")),
        clock.clone(),
    );
    module.sample_now(&ctx).unwrap();
    run_seconds(&module, &ctx, &clock, 2);
    module.apply_config(AppUsageConfig {
        keep_window_titles: false,
        ..Default::default()
    });
    run_seconds(&module, &ctx, &clock, 2);
    module.tracker.finish(&ctx).unwrap();
    let titles: i64 = ctx.db.with_conn(|conn| {
        Ok(conn.query_row("SELECT (SELECT COUNT(*) FROM tracked_apps WHERE window_title IS NOT NULL) + (SELECT COUNT(*) FROM app_usage_sessions WHERE window_title IS NOT NULL)", [], |row| row.get(0))?)
    }).unwrap();
    assert_eq!(titles, 0);
    assert_eq!(total_seconds(&ctx), 4);
}

#[test]
fn failed_session_write_is_atomic_and_retried_once() {
    let dir = temp_dir("session-retry");
    let ctx = context(dir.path(), true);
    let watcher = Arc::new(FakeWatcher::new("alpha", "Alpha"));
    let clock = Arc::new(FakeClock::at(1_790_000_000_000));
    let module = AppUsageModule::with_clock(watcher.clone(), clock.clone());
    module.sample_now(&ctx).unwrap();
    run_seconds(&module, &ctx, &clock, 5);
    sql(&ctx, "CREATE TEMP TRIGGER fail_session BEFORE INSERT ON app_usage_sessions BEGIN SELECT RAISE(ABORT, 'session unavailable'); END;");
    watcher.set_app("beta", "Beta");
    assert!(module.sample_now(&ctx).is_err());
    assert_eq!(
        total_seconds(&ctx),
        0,
        "hourly writes must roll back with sessions"
    );
    sql(&ctx, "DROP TRIGGER fail_session;");
    module.flush_pending(&ctx).unwrap();
    module.flush_pending(&ctx).unwrap();
    assert_eq!(total_seconds(&ctx), 5);
    let sessions: i64 = ctx
        .db
        .with_conn(|conn| {
            Ok(
                conn.query_row("SELECT COUNT(*) FROM app_usage_sessions", [], |row| {
                    row.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(sessions, 1);
}

#[test]
fn failed_disable_recovers_sampling_and_preserves_unflushed_usage() {
    let dir = temp_dir("disable-retry");
    let clock = Arc::new(FakeClock::at(1_790_000_000_000));
    let module = Arc::new(AppUsageModule::with_clock(
        Arc::new(FakeWatcher::new("editor", "Editor")),
        clock.clone(),
    ));
    let kernel = app_core::AppKernelBuilder::new(dir.path())
        .with_in_memory_db(true)
        .register_module_arc(module.clone())
        .unwrap()
        .bootstrap()
        .unwrap();
    let ctx = kernel.context();
    module.sample_now(ctx).unwrap();
    run_seconds(&module, ctx, &clock, 5);
    sql(ctx, "CREATE TEMP TRIGGER fail_usage BEFORE INSERT ON app_usage_hourly BEGIN SELECT RAISE(ABORT, 'usage unavailable'); END;");
    assert!(kernel.set_module_enabled("app_usage", false).is_err());
    assert!(kernel.registry().is_enabled("app_usage"));
    assert!(module.is_tracking());
    sql(ctx, "DROP TRIGGER fail_usage;");
    module.flush_pending(ctx).unwrap();
    assert_eq!(total_seconds(ctx), 5);
    kernel.shutdown().unwrap();
    assert!(!module.is_tracking());
}

#[test]
fn accepted_wall_clock_drift_does_not_change_billed_duration() {
    let dir = temp_dir("clock-drift");
    let ctx = context(dir.path(), true);
    let clock = Arc::new(FakeClock::at(1_790_000_000_000));
    let module = AppUsageModule::with_clock(
        Arc::new(FakeWatcher::new("editor", "Editor")),
        clock.clone(),
    );
    module.sample_now(&ctx).unwrap();
    clock.mono.fetch_add(1_000, Ordering::SeqCst);
    clock.utc.fetch_add(2_000, Ordering::SeqCst);
    module.sample_now(&ctx).unwrap();
    module.tracker.finish(&ctx).unwrap();
    assert_eq!(total_seconds(&ctx), 1);
}

#[test]
fn half_hour_timezone_splits_at_local_midnight() {
    let start = chrono::DateTime::parse_from_rfc3339("2026-10-25T18:29:59Z")
        .unwrap()
        .timestamp_millis();
    let slices = crate::tracker::split_interval(start, start + 2_000, |_| 19_800);
    assert_eq!(slices.len(), 2);
    assert_eq!(
        (&*slices[0].date, slices[0].hour, slices[0].millis),
        ("2026-10-25", 23, 1_000)
    );
    assert_eq!(
        (&*slices[1].date, slices[1].hour, slices[1].millis),
        ("2026-10-26", 0, 1_000)
    );
}
