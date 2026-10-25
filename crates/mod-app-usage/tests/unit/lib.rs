use super::*;
use crate::tracker::UsageClock;
use app_core::{
    AppConfig, DbPool, EnvProbeService, EventBus, HeadlessWindowBackend, ModuleContext,
    ShortcutRegistry, TargetOs, WindowPoolManager,
};
use os_hal::{ActiveAppSnapshot, ActiveAppWatcher, HalResult};
use parking_lot::{Mutex, RwLock};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::TempDir;

#[path = "reliability.rs"]
mod reliability;

#[derive(Default)]
struct FakeClock {
    mono: AtomicU64,
    utc: AtomicI64,
}

impl FakeClock {
    fn at(utc_ms: i64) -> Self {
        Self {
            mono: AtomicU64::new(0),
            utc: AtomicI64::new(utc_ms),
        }
    }

    fn advance(&self, millis: u64) {
        self.mono.fetch_add(millis, Ordering::SeqCst);
        self.utc.fetch_add(millis as i64, Ordering::SeqCst);
    }
}

impl UsageClock for FakeClock {
    fn monotonic_millis(&self) -> u64 {
        self.mono.load(Ordering::SeqCst)
    }

    fn utc_millis(&self) -> i64 {
        self.utc.load(Ordering::SeqCst)
    }
}

struct FakeWatcher {
    app: Mutex<Option<ActiveAppSnapshot>>,
    idle_seconds: AtomicU64,
    locked: AtomicBool,
    audio_playing: AtomicBool,
    audio_supported: AtomicBool,
    event_starts: AtomicUsize,
    event_stops: AtomicUsize,
}

impl FakeWatcher {
    fn new(app_id: &str, name: &str) -> Self {
        Self {
            app: Mutex::new(Some(ActiveAppSnapshot {
                app_id: app_id.to_string(),
                app_name: name.to_string(),
                window_title: format!("{name} window"),
                executable_path: Some(format!("/Applications/{name}.app")),
                process_id: 42,
            })),
            idle_seconds: AtomicU64::new(0),
            locked: AtomicBool::new(false),
            audio_playing: AtomicBool::new(false),
            audio_supported: AtomicBool::new(true),
            event_starts: AtomicUsize::new(0),
            event_stops: AtomicUsize::new(0),
        }
    }

    fn set_app(&self, app_id: &str, name: &str) {
        *self.app.lock() = Some(ActiveAppSnapshot {
            app_id: app_id.to_string(),
            app_name: name.to_string(),
            window_title: format!("{name} window"),
            executable_path: Some(format!("/Applications/{name}.app")),
            process_id: 42,
        });
    }

    fn set_idle(&self, seconds: u64) {
        self.idle_seconds.store(seconds, Ordering::SeqCst);
    }

    fn set_locked(&self, locked: bool) {
        self.locked.store(locked, Ordering::SeqCst);
    }

    fn set_audio_playing(&self, playing: bool) {
        self.audio_playing.store(playing, Ordering::SeqCst);
    }

    fn set_audio_supported(&self, supported: bool) {
        self.audio_supported.store(supported, Ordering::SeqCst);
    }
}

impl ActiveAppWatcher for FakeWatcher {
    fn current_active_app(&self) -> HalResult<ActiveAppSnapshot> {
        self.app
            .lock()
            .clone()
            .ok_or_else(|| os_hal::HalError::NotFound("no app".to_string()))
    }

    fn system_idle_seconds(&self) -> HalResult<u64> {
        Ok(self.idle_seconds.load(Ordering::SeqCst))
    }

    fn is_screen_locked(&self) -> HalResult<bool> {
        Ok(self.locked.load(Ordering::SeqCst))
    }

    fn default_output_audio_active(&self) -> HalResult<bool> {
        if self.audio_supported.load(Ordering::SeqCst) {
            Ok(self.audio_playing.load(Ordering::SeqCst))
        } else {
            Err(os_hal::HalError::Unsupported("audio"))
        }
    }

    fn start_usage_event_monitor(&self) -> HalResult<()> {
        self.event_starts.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn stop_usage_event_monitor(&self) {
        self.event_stops.fetch_add(1, Ordering::SeqCst);
    }
}

fn context(dir: &Path, in_memory: bool) -> ModuleContext {
    let db = if in_memory {
        DbPool::open_in_memory().unwrap()
    } else {
        DbPool::open(dir.join("app.db")).unwrap()
    };
    db.run_migrations(&crate::store::migrations()).unwrap();
    ModuleContext {
        app_data_dir: dir.to_path_buf(),
        db,
        event_bus: EventBus::default(),
        config: Arc::new(RwLock::new(AppConfig::default())),
        env_probe: Arc::new(EnvProbeService::new()),
        window_pool: Arc::new(WindowPoolManager::new(Arc::new(HeadlessWindowBackend))),
        shortcut_registry: Arc::new(ShortcutRegistry::new(TargetOs::current())),
    }
}

fn temp_dir(label: &str) -> TempDir {
    tempfile::Builder::new()
        .prefix(&format!("app-usage-{label}-"))
        .tempdir()
        .unwrap()
}

fn run_seconds(module: &AppUsageModule, ctx: &ModuleContext, clock: &FakeClock, seconds: u64) {
    for _ in 0..seconds {
        clock.advance(1_000);
        module.sample_now(ctx).unwrap();
    }
}

fn total_seconds(ctx: &ModuleContext) -> u64 {
    ctx.db
        .with_conn(|conn| {
            conn.query_row(
                "SELECT COALESCE(SUM(active_seconds), 0) FROM app_usage_hourly",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value.max(0) as u64)
            .map_err(Into::into)
        })
        .unwrap()
}

#[test]
fn active_time_is_accumulated_with_monotonic_clock() {
    let dir = temp_dir("accumulate");
    let ctx = context(dir.path(), true);
    let watcher = Arc::new(FakeWatcher::new("com.example.editor", "Editor"));
    // Align to a whole second so per-hour integer reporting does not depend on
    // the test's wall-clock start falling across a fractional-second boundary.
    let clock = Arc::new(FakeClock::at(chrono::Utc::now().timestamp() * 1_000));
    let module = AppUsageModule::with_clock(watcher.clone(), clock.clone());

    module.sample_now(&ctx).unwrap();
    run_seconds(&module, &ctx, &clock, 60);

    assert_eq!(total_seconds(&ctx), 60);
    let overview = module
        .overview(&ctx, &module.today_local(), &module.today_local())
        .unwrap();
    assert_eq!(overview.apps.len(), 1);
    assert_eq!(overview.apps[0].app_id, "com.example.editor");
    assert_eq!(overview.apps[0].active_seconds, 60);
}

#[test]
fn idle_and_lock_pause_without_recording() {
    let dir = temp_dir("idle");
    let ctx = context(dir.path(), true);
    let watcher = Arc::new(FakeWatcher::new("com.example.editor", "Editor"));
    let clock = Arc::new(FakeClock::at(chrono::Utc::now().timestamp_millis()));
    let module = AppUsageModule::with_clock(watcher.clone(), clock.clone());

    module.sample_now(&ctx).unwrap();
    run_seconds(&module, &ctx, &clock, 5);

    watcher.set_idle(301);
    run_seconds(&module, &ctx, &clock, 10);
    module.flush_pending(&ctx).unwrap();
    assert_eq!(total_seconds(&ctx), 6);

    watcher.set_idle(0);
    watcher.set_locked(true);
    run_seconds(&module, &ctx, &clock, 10);
    module.flush_pending(&ctx).unwrap();
    assert_eq!(total_seconds(&ctx), 6);

    watcher.set_locked(false);
    run_seconds(&module, &ctx, &clock, 5);
    module.flush_pending(&ctx).unwrap();
    assert_eq!(total_seconds(&ctx), 10);
}

#[test]
fn sleep_wake_gap_is_not_counted() {
    let dir = temp_dir("sleep");
    let ctx = context(dir.path(), true);
    let watcher = Arc::new(FakeWatcher::new("com.example.editor", "Editor"));
    let clock = Arc::new(FakeClock::at(chrono::Utc::now().timestamp_millis()));
    let module = AppUsageModule::with_clock(watcher.clone(), clock.clone());

    module.sample_now(&ctx).unwrap();
    run_seconds(&module, &ctx, &clock, 10);
    clock.advance(30_000);
    module.sample_now(&ctx).unwrap();
    run_seconds(&module, &ctx, &clock, 5);
    module.flush_pending(&ctx).unwrap();

    assert_eq!(total_seconds(&ctx), 15);
}

#[test]
fn focus_switch_flushes_and_splits_across_hour_boundary() {
    let dir = temp_dir("hour");
    let ctx = context(dir.path(), true);
    let watcher = Arc::new(FakeWatcher::new("app.alpha", "Alpha"));
    let start = chrono::DateTime::parse_from_rfc3339("2026-10-25T23:59:30Z")
        .unwrap()
        .timestamp_millis();
    let clock = Arc::new(FakeClock::at(start));
    let module = AppUsageModule::with_clock(watcher.clone(), clock.clone());

    module.sample_now(&ctx).unwrap();
    run_seconds(&module, &ctx, &clock, 30);
    watcher.set_app("app.beta", "Beta");
    run_seconds(&module, &ctx, &clock, 10);
    module.flush_pending(&ctx).unwrap();

    let rows: Vec<(String, i64, String, i64)> = ctx
        .db
        .with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT date_str, hour_of_day, app_id, active_seconds
                 FROM app_usage_hourly ORDER BY date_str, hour_of_day, app_id",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?;
            let mut result = Vec::new();
            for row in rows {
                result.push(row?);
            }
            Ok(result)
        })
        .unwrap();

    let total: i64 = rows.iter().map(|row| row.3).sum();
    assert_eq!(total, 40);
    let alpha_total: i64 = rows
        .iter()
        .filter(|row| row.2 == "app.alpha")
        .map(|row| row.3)
        .sum();
    let beta_total: i64 = rows
        .iter()
        .filter(|row| row.2 == "app.beta")
        .map(|row| row.3)
        .sum();
    assert_eq!(alpha_total, 31);
    assert_eq!(beta_total, 9);
    let alpha_hours: Vec<i64> = rows
        .iter()
        .filter(|row| row.2 == "app.alpha")
        .map(|row| row.1)
        .collect();
    assert_eq!(alpha_hours.len(), 2, "alpha should span two local hours");
}

#[test]
fn ignored_apps_stop_counting_but_keep_history() {
    let dir = temp_dir("ignored");
    let ctx = context(dir.path(), true);
    let watcher = Arc::new(FakeWatcher::new("com.example.editor", "Editor"));
    let clock = Arc::new(FakeClock::at(chrono::Utc::now().timestamp_millis()));
    let module = AppUsageModule::with_clock(watcher.clone(), clock.clone());

    module.sample_now(&ctx).unwrap();
    run_seconds(&module, &ctx, &clock, 5);
    module.sample_now(&ctx).unwrap();
    // 先通过切换应用触发刷写，确保应用已进入 tracked_apps。
    watcher.set_app("com.example.other", "Other");
    run_seconds(&module, &ctx, &clock, 1);
    watcher.set_app("com.example.editor", "Editor");
    run_seconds(&module, &ctx, &clock, 1);
    module
        .set_app_category(&ctx, "com.example.editor", "work", true)
        .unwrap();
    run_seconds(&module, &ctx, &clock, 5);
    module.flush_pending(&ctx).unwrap();

    let editor_seconds = ctx
        .db
        .with_conn(|conn| {
            conn.query_row(
                "SELECT COALESCE(SUM(active_seconds), 0) FROM app_usage_hourly
                 WHERE app_id = 'com.example.editor'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(Into::into)
        })
        .unwrap();
    assert_eq!(editor_seconds, 7);
    let apps = module.list_tracked_apps(&ctx).unwrap();
    let editor = apps
        .iter()
        .find(|app| app.app_id == "com.example.editor")
        .unwrap();
    assert!(editor.ignored);
    assert_eq!(editor.category, "work");
}

#[test]
fn restart_does_not_recount_or_add_offline_time() {
    let dir = temp_dir("restart");
    let clock = Arc::new(FakeClock::at(1_790_000_000_000));
    let watcher = Arc::new(FakeWatcher::new("com.example.editor", "Editor"));

    {
        let ctx = context(dir.path(), false);
        let module = AppUsageModule::with_clock(watcher.clone(), clock.clone());
        module.sample_now(&ctx).unwrap();
        run_seconds(&module, &ctx, &clock, 10);
        module.tracker.finish(&ctx).unwrap();
    }

    clock.advance(3_600_000);
    {
        let ctx = context(dir.path(), false);
        let module = AppUsageModule::with_clock(watcher.clone(), clock.clone());
        module.sample_now(&ctx).unwrap();
        run_seconds(&module, &ctx, &clock, 5);
        module.flush_pending(&ctx).unwrap();
        assert_eq!(total_seconds(&ctx), 15);
    }
}

#[test]
fn window_titles_can_be_disabled_before_persisting() {
    let dir = temp_dir("titles");
    let ctx = context(dir.path(), true);
    let watcher = Arc::new(FakeWatcher::new("com.example.editor", "Editor"));
    let clock = Arc::new(FakeClock::at(chrono::Utc::now().timestamp_millis()));
    let module = AppUsageModule::with_clock(watcher.clone(), clock.clone());
    module.apply_config(crate::config::AppUsageConfig {
        keep_window_titles: false,
        ..Default::default()
    });
    module.sample_now(&ctx).unwrap();
    run_seconds(&module, &ctx, &clock, 2);
    module.tracker.finish(&ctx).unwrap();

    let title: Option<String> = ctx
        .db
        .with_conn(|conn| {
            conn.query_row(
                "SELECT window_title FROM tracked_apps WHERE app_id = 'com.example.editor'",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
        })
        .unwrap();
    assert!(title.is_none());
}

#[test]
fn capabilities_report_audio_limit_when_probe_is_unsupported() {
    let watcher = Arc::new(FakeWatcher::new("com.example.editor", "Editor"));
    watcher.set_audio_supported(false);
    let module = AppUsageModule::new(watcher);
    let caps = module.capabilities();
    assert!(caps.active_app);
    assert!(caps.idle_detection);
    assert!(caps.screen_lock_detection);
    assert!(!caps.audio_afk_exception);
    assert!(caps.limitations.iter().any(|line| line.contains("音频")));
}

#[test]
fn audio_playback_keeps_idle_time_active() {
    let dir = temp_dir("audio-afk");
    let ctx = context(dir.path(), true);
    let watcher = Arc::new(FakeWatcher::new("com.example.player", "Player"));
    watcher.set_idle(301);
    watcher.set_audio_playing(true);
    let clock = Arc::new(FakeClock::at(chrono::Utc::now().timestamp_millis()));
    let module = AppUsageModule::with_clock(watcher.clone(), clock.clone());

    module.sample_now(&ctx).unwrap();
    run_seconds(&module, &ctx, &clock, 10);
    module.flush_pending(&ctx).unwrap();
    assert_eq!(total_seconds(&ctx), 10);

    // 停止播放后的那一个间隔仍处于上一采样点活跃状态，随后进入 AFK 暂停。
    watcher.set_audio_playing(false);
    run_seconds(&module, &ctx, &clock, 10);
    module.flush_pending(&ctx).unwrap();
    assert_eq!(total_seconds(&ctx), 11);
}

#[test]
fn lock_and_sleep_take_priority_over_audio_exception() {
    let dir = temp_dir("audio-lock-priority");
    let ctx = context(dir.path(), true);
    let watcher = Arc::new(FakeWatcher::new("com.example.player", "Player"));
    watcher.set_idle(301);
    watcher.set_audio_playing(true);
    watcher.set_locked(true);
    let clock = Arc::new(FakeClock::at(chrono::Utc::now().timestamp_millis()));
    let module = AppUsageModule::with_clock(watcher.clone(), clock.clone());

    module.sample_now(&ctx).unwrap();
    run_seconds(&module, &ctx, &clock, 10);
    module.flush_pending(&ctx).unwrap();
    assert_eq!(total_seconds(&ctx), 0);

    watcher.set_locked(false);
    // 解锁后的第一个采样点只负责重新开启会话，随后 9 个间隔计入时长。
    run_seconds(&module, &ctx, &clock, 10);
    module.flush_pending(&ctx).unwrap();
    assert_eq!(total_seconds(&ctx), 9);
}

#[test]
fn disabling_audio_exception_restores_plain_idle_pause() {
    let dir = temp_dir("audio-disabled");
    let ctx = context(dir.path(), true);
    let watcher = Arc::new(FakeWatcher::new("com.example.player", "Player"));
    watcher.set_idle(301);
    watcher.set_audio_playing(true);
    let clock = Arc::new(FakeClock::at(chrono::Utc::now().timestamp_millis()));
    let module = AppUsageModule::with_clock(watcher.clone(), clock.clone());
    module.apply_config(crate::config::AppUsageConfig {
        audio_afk_exception: false,
        ..Default::default()
    });

    module.sample_now(&ctx).unwrap();
    run_seconds(&module, &ctx, &clock, 10);
    module.flush_pending(&ctx).unwrap();
    assert_eq!(total_seconds(&ctx), 0);
    let caps = module.capabilities();
    assert!(caps.audio_afk_exception);
    assert!(caps.limitations.iter().any(|line| line.contains("关闭")));
}

#[test]
fn repeated_enable_disable_starts_and_stops_event_monitor() {
    use app_core::AppModule as _;

    let dir = temp_dir("event-lifecycle");
    let ctx = context(dir.path(), true);
    let watcher = Arc::new(FakeWatcher::new("com.example.editor", "Editor"));
    let clock = Arc::new(FakeClock::at(chrono::Utc::now().timestamp_millis()));
    let module = AppUsageModule::with_clock(watcher.clone(), clock.clone());

    module.on_init(&ctx).unwrap();
    module.on_shutdown(&ctx).unwrap();
    module.on_init(&ctx).unwrap();
    module.on_shutdown(&ctx).unwrap();

    assert_eq!(watcher.event_starts.load(Ordering::SeqCst), 2);
    assert_eq!(watcher.event_stops.load(Ordering::SeqCst), 2);
}

#[test]
fn sessions_list_reports_day_sessions_and_hides_titles_when_disabled() {
    let dir = temp_dir("sessions");
    let ctx = context(dir.path(), true);
    let clock = Arc::new(FakeClock::at(chrono::Utc::now().timestamp_millis()));
    let module = AppUsageModule::with_clock(
        Arc::new(FakeWatcher::new("com.example.editor", "Editor")),
        clock.clone(),
    );

    module.sample_now(&ctx).unwrap();
    run_seconds(&module, &ctx, &clock, 5);
    module.tracker.finish(&ctx).unwrap();

    let today = module.today_local();
    let page = module.sessions(&ctx, &today, 0, 20).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.sessions[0].app_id, "com.example.editor");
    assert_eq!(page.sessions[0].active_seconds, 5);
    assert!(page.sessions[0].window_title.is_some());

    module.apply_config(crate::config::AppUsageConfig {
        keep_window_titles: false,
        ..Default::default()
    });
    let hidden = module.sessions(&ctx, &today, 0, 20).unwrap();
    assert_eq!(hidden.total, 1);
    assert!(hidden.sessions[0].window_title.is_none());
}

#[test]
fn module_shutdown_flushes_pending_usage_and_stops_sampler() {
    use app_core::AppModule as _;

    let dir = temp_dir("shutdown");
    let ctx = context(dir.path(), true);
    let watcher = Arc::new(FakeWatcher::new("com.example.editor", "Editor"));
    let clock = Arc::new(FakeClock::at(chrono::Utc::now().timestamp_millis()));
    let module = AppUsageModule::with_clock(watcher, clock.clone());

    module.on_init(&ctx).unwrap();
    module.sample_now(&ctx).unwrap();
    run_seconds(&module, &ctx, &clock, 5);
    assert_eq!(total_seconds(&ctx), 0, "not flushed before shutdown");
    module.on_shutdown(&ctx).unwrap();

    assert!(!module.is_tracking());
    assert_eq!(total_seconds(&ctx), 5);
    let sessions: i64 = ctx
        .db
        .with_conn(|conn| {
            conn.query_row("SELECT COUNT(*) FROM app_usage_sessions", [], |row| {
                row.get(0)
            })
            .map_err(Into::into)
        })
        .unwrap();
    assert_eq!(sessions, 1);
}
