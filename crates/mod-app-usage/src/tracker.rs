//! 前台采样与单调时钟计时核心。
//!
//! 计时规则：
//! - 时长来自单调时钟，日期/小时桶来自 UTC 墙钟；在本地小时及 UTC 小时边界切段。
//! - 采样间隔异常（休眠唤醒、系统时钟跳变）不补记离线时间，并关闭当前会话。
//! - 锁屏、息屏、睡眠、键鼠空闲超过阈值时不累计；锁屏/睡眠优先于音频例外。
//! - 键鼠空闲但默认输出设备仍在播放音频时，设置允许则继续计时（视频/音乐场景）。
//! - 进程重启只从第一个采样点开始计时，不会把停机时间计入，也不会恢复旧的未刷写增量。

use crate::config::AppUsageConfig;
use crate::report::AppUsageCapabilities;
use crate::store::{self, AppMeta, ClosedSession, HourlyDelta};
use app_core::{CoreResult, ModuleContext};
use chrono::{Datelike, TimeZone, Timelike, Utc};
use os_hal::ActiveAppWatcher;
use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use tracing::debug;

pub const SAMPLE_INTERVAL_MS: u64 = 1_000;
const MAX_SAMPLE_GAP_MS: u64 = 5_000;
const CLOCK_DRIFT_TOLERANCE_MS: i64 = 2_000;
const HOUR_MS: i64 = 3_600_000;

/// 可注入时间源，测试用固定/步进时钟验证切段和休眠跳变。
pub trait UsageClock: Send + Sync + 'static {
    /// 不受系统时间调整影响的单调毫秒数。
    fn monotonic_millis(&self) -> u64;
    /// 当前 UTC Unix 毫秒；只用于日期/小时归属。
    fn utc_millis(&self) -> i64;
}

/// 生产时钟：单调部分基于 `Instant`，墙钟基于系统 UTC。
pub struct SystemUsageClock {
    start: Instant,
}

impl SystemUsageClock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for SystemUsageClock {
    fn default() -> Self {
        Self::new()
    }
}

impl UsageClock for SystemUsageClock {
    fn monotonic_millis(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn utc_millis(&self) -> i64 {
        Utc::now().timestamp_millis()
    }
}

/// 单个小时桶的归属片段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageSlice {
    pub date: String,
    pub hour: u8,
    pub millis: u64,
}

/// 在本地及 UTC 小时边界切段，再用偏移函数映射为本地日期/小时。
///
/// 让测试注入偏移变化即可覆盖时区与夏令时；生产环境传入系统本地偏移。
pub fn split_interval(
    start_ms: i64,
    end_ms: i64,
    offset_at: impl Fn(i64) -> i32,
) -> Vec<UsageSlice> {
    if end_ms <= start_ms {
        return Vec::new();
    }
    let mut slices = Vec::new();
    let mut cursor = start_ms;
    while cursor < end_ms {
        let offset_ms = i64::from(offset_at(cursor)) * 1_000;
        let next_utc_hour = cursor.div_euclid(HOUR_MS).saturating_add(1) * HOUR_MS;
        let next_local_hour =
            (cursor + offset_ms).div_euclid(HOUR_MS).saturating_add(1) * HOUR_MS - offset_ms;
        let chunk_end = next_utc_hour.min(next_local_hour).min(end_ms);
        let local = Utc
            .timestamp_millis_opt(cursor.saturating_add(offset_ms))
            .single();
        if let Some(local) = local {
            slices.push(UsageSlice {
                date: format!(
                    "{:04}-{:02}-{:02}",
                    local.year(),
                    local.month(),
                    local.day()
                ),
                hour: local.hour() as u8,
                millis: (chunk_end - cursor) as u64,
            });
        }
        cursor = chunk_end;
    }
    slices
}

fn timed_slices(start: i64, end: i64, elapsed: u64) -> Vec<UsageSlice> {
    let mut slices = split_interval(start, end, local_offset_seconds);
    let wall = (end - start) as u128;
    let mut cumulative_wall = 0u128;
    let mut assigned = 0u64;
    for slice in &mut slices {
        cumulative_wall += u128::from(slice.millis);
        let cumulative_elapsed = (cumulative_wall * u128::from(elapsed) / wall) as u64;
        slice.millis = cumulative_elapsed - assigned;
        assigned = cumulative_elapsed;
    }
    slices
}

fn local_offset_seconds(utc_ms: i64) -> i32 {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(utc_ms)
        .single()
        .map(|value| value.offset().local_minus_utc())
        .unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BucketKey {
    date: String,
    hour: u8,
    app_id: String,
}

#[derive(Debug, Clone)]
struct Session {
    app: AppMeta,
    started_at: i64,
    last_at: i64,
    active_ms: u64,
}

#[derive(Default)]
struct TrackerState {
    last_mono: Option<u64>,
    last_utc: Option<i64>,
    last_flush_mono: u64,
    active: bool,
    app: Option<AppMeta>,
    session: Option<Session>,
    pending: HashMap<BucketKey, u64>,
    pending_apps: HashMap<String, AppMeta>,
    pending_sessions: Vec<ClosedSession>,
}

impl TrackerState {
    fn close_session(&mut self) -> Option<ClosedSession> {
        let session = self.session.take()?;
        if session.active_ms < 1_000 {
            return None;
        }
        Some(ClosedSession {
            app: session.app,
            started_at: session.started_at,
            ended_at: session.last_at,
            active_seconds: session.active_ms / 1_000,
        })
    }
}

/// 计时状态机；不直接认识 Tauri 或宿主。
pub(crate) struct Tracker {
    watcher: Arc<dyn ActiveAppWatcher>,
    clock: Arc<dyn UsageClock>,
    config: RwLock<AppUsageConfig>,
    ignored: RwLock<HashSet<String>>,
    state: Mutex<TrackerState>,
    operation: Mutex<()>,
    #[cfg(test)]
    write_attempts: std::sync::atomic::AtomicUsize,
}

impl Tracker {
    pub(crate) fn new(watcher: Arc<dyn ActiveAppWatcher>, clock: Arc<dyn UsageClock>) -> Self {
        Self {
            watcher,
            clock,
            config: RwLock::new(AppUsageConfig::default()),
            ignored: RwLock::new(HashSet::new()),
            state: Mutex::new(TrackerState::default()),
            operation: Mutex::new(()),
            #[cfg(test)]
            write_attempts: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    #[cfg(test)]
    pub(crate) fn write_attempts(&self) -> usize {
        self.write_attempts
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(crate) fn apply_config(&self, config: AppUsageConfig) {
        let _operation = self.operation.lock();
        let config = config.normalized();
        if !config.keep_window_titles {
            let mut state = self.state.lock();
            if let Some(app) = state.app.as_mut() {
                app.window_title = None;
            }
            if let Some(session) = state.session.as_mut() {
                session.app.window_title = None;
            }
            for app in state.pending_apps.values_mut() {
                app.window_title = None;
            }
            for session in &mut state.pending_sessions {
                session.app.window_title = None;
            }
        }
        *self.config.write() = config;
    }

    pub(crate) fn config(&self) -> AppUsageConfig {
        self.config.read().clone()
    }

    pub(crate) fn set_ignored(&self, app_id: &str, ignored: bool) {
        let mut ignored_set = self.ignored.write();
        if ignored {
            ignored_set.insert(app_id.to_string());
        } else {
            ignored_set.remove(app_id);
        }
    }

    pub(crate) fn begin(&self) {
        let _operation = self.operation.lock();
        let mut state = self.state.lock();
        state.last_mono = None;
        state.last_utc = None;
        state.last_flush_mono = self.clock.monotonic_millis();
        state.active = false;
        state.app = None;
        state.session = None;
    }

    pub(crate) fn capabilities(&self) -> AppUsageCapabilities {
        let mut limitations = Vec::new();
        let active_app = self.watcher.current_active_app().is_ok();
        if !active_app {
            limitations.push("当前系统无法读取前台应用，采样不会产生数据。".to_string());
        }
        let idle_ok = self.watcher.system_idle_seconds().is_ok();
        if !idle_ok {
            limitations.push("当前系统无法检测真实键鼠空闲，采样暂停以避免误记时。".to_string());
        }
        let lock_ok = self.watcher.is_screen_locked().is_ok();
        if !lock_ok {
            limitations
                .push("当前系统无法检测真实锁屏/息屏状态，采样暂停以避免误记时。".to_string());
        }
        let audio_ok = self.watcher.default_output_audio_active().is_ok();
        if !audio_ok {
            limitations
                .push("当前系统无法检测默认输出设备的音频活动，音频 AFK 例外不可用。".to_string());
        } else if !self.config().audio_afk_exception {
            limitations.push("音频 AFK 例外已在设置中关闭。".to_string());
        }
        AppUsageCapabilities {
            active_app,
            idle_detection: idle_ok,
            screen_lock_detection: lock_ok,
            audio_afk_exception: audio_ok,
            limitations,
        }
    }

    fn is_ignored(&self, app_id: &str) -> bool {
        self.ignored.read().contains(app_id)
    }

    /// 采集一次前台状态。测试可直接用注入时钟推进后重复调用。
    pub(crate) fn sample(&self, ctx: &ModuleContext) -> CoreResult<()> {
        let _operation = self.operation.lock();
        let now_mono = self.clock.monotonic_millis();
        let now_utc = self.clock.utc_millis();
        let config = self.config.read().clone();

        let idle_seconds = self.watcher.system_idle_seconds().ok();
        let locked = self.watcher.is_screen_locked().ok();
        // 只有键鼠已达到空闲阈值时才探测音频，避免正常操作期间反复查询系统设备。
        let idle_afk = idle_seconds.is_some_and(|idle| idle >= config.idle_threshold_secs);
        let audio_playing = idle_afk
            && config.audio_afk_exception
            && self.watcher.default_output_audio_active().unwrap_or(false);
        let snapshot = self.watcher.current_active_app().ok();
        let app = snapshot
            .filter(|snapshot| !snapshot.app_id.trim().is_empty())
            .map(|snapshot| AppMeta {
                app_id: snapshot.app_id,
                app_name: snapshot.app_name,
                executable_path: snapshot.executable_path,
                window_title: config
                    .keep_window_titles
                    .then_some(snapshot.window_title)
                    .filter(|title| !title.trim().is_empty()),
            })
            .filter(|app| !self.is_ignored(&app.app_id));
        // 锁屏/睡眠优先：即使音频正在播放也不计时。未知空闲/锁定状态一律暂停。
        let should_track = app.is_some()
            && locked == Some(false)
            && idle_seconds.is_some_and(|idle| idle < config.idle_threshold_secs || audio_playing);

        let mut closed_session: Option<ClosedSession> = None;
        let mut flush_requested = false;
        {
            let mut state = self.state.lock();

            if let (Some(prev_mono), Some(prev_utc)) = (state.last_mono, state.last_utc) {
                let mono_delta = now_mono.saturating_sub(prev_mono);
                let wall_delta = now_utc.saturating_sub(prev_utc);
                let sane = mono_delta <= MAX_SAMPLE_GAP_MS
                    && wall_delta > 0
                    && (wall_delta - mono_delta as i64).abs() <= CLOCK_DRIFT_TOLERANCE_MS;
                if sane {
                    if state.active {
                        if let Some(previous_app) = state.app.clone() {
                            let slices = timed_slices(prev_utc, now_utc, mono_delta);
                            for slice in slices {
                                let key = BucketKey {
                                    date: slice.date,
                                    hour: slice.hour,
                                    app_id: previous_app.app_id.clone(),
                                };
                                *state.pending.entry(key).or_insert(0) += slice.millis;
                            }
                            state
                                .pending_apps
                                .insert(previous_app.app_id.clone(), previous_app);
                            if let Some(session) = state.session.as_mut() {
                                session.active_ms += mono_delta;
                                session.last_at = now_utc;
                            }
                        }
                    }
                } else {
                    // 休眠唤醒或系统时钟跳变：丢弃这段间隔，绝不补记离线时间。
                    debug!(
                        mono_delta,
                        wall_delta, "Skipped usage interval after clock jump/wake"
                    );
                    closed_session = state.close_session();
                    state.active = false;
                    state.app = None;
                    flush_requested = true;
                }
            }

            let same_app =
                state.app.as_ref().map(|app| &app.app_id) == app.as_ref().map(|app| &app.app_id);
            if state.active && should_track && same_app {
                // 同一应用持续活跃，保留会话。
            } else {
                let was_active = state.active;
                if let Some(session) = state.close_session() {
                    closed_session = Some(session);
                    flush_requested = true;
                }
                state.active = should_track;
                state.app = app.clone();
                if should_track {
                    if let Some(app) = app {
                        state.pending_apps.insert(app.app_id.clone(), app.clone());
                        state.session = Some(Session {
                            app,
                            started_at: now_utc,
                            last_at: now_utc,
                            active_ms: 0,
                        });
                    }
                    flush_requested = true;
                } else if was_active {
                    flush_requested = true;
                }
            }

            state.last_mono = Some(now_mono);
            state.last_utc = Some(now_utc);
            if now_mono.saturating_sub(state.last_flush_mono) >= config.flush_interval_millis() {
                flush_requested = true;
            }
        }

        if let Some(session) = closed_session {
            self.state.lock().pending_sessions.push(session);
        }
        if flush_requested {
            self.flush(ctx, now_mono)?;
        }
        Ok(())
    }

    /// 立即刷写已结算的整秒增量（测试与显式刷新使用）。
    pub(crate) fn flush_now(&self, ctx: &ModuleContext) -> CoreResult<()> {
        let _operation = self.operation.lock();
        let now = self.clock.monotonic_millis();
        self.flush(ctx, now)
    }

    /// 停止采样前结算最后一段活动并刷写。
    pub(crate) fn finish(&self, ctx: &ModuleContext) -> CoreResult<()> {
        let _operation = self.operation.lock();
        let now_mono = self.clock.monotonic_millis();
        let now_utc = self.clock.utc_millis();
        let closed_session = {
            let mut state = self.state.lock();
            if let (Some(prev_mono), Some(prev_utc)) = (state.last_mono, state.last_utc) {
                let mono_delta = now_mono.saturating_sub(prev_mono);
                let wall_delta = now_utc.saturating_sub(prev_utc);
                let sane = mono_delta <= MAX_SAMPLE_GAP_MS
                    && wall_delta > 0
                    && (wall_delta - mono_delta as i64).abs() <= CLOCK_DRIFT_TOLERANCE_MS;
                if sane && state.active {
                    if let Some(previous_app) = state.app.clone() {
                        let slices = timed_slices(prev_utc, now_utc, mono_delta);
                        for slice in slices {
                            let key = BucketKey {
                                date: slice.date,
                                hour: slice.hour,
                                app_id: previous_app.app_id.clone(),
                            };
                            *state.pending.entry(key).or_insert(0) += slice.millis;
                        }
                        state
                            .pending_apps
                            .insert(previous_app.app_id.clone(), previous_app);
                        if let Some(session) = state.session.as_mut() {
                            session.active_ms += mono_delta;
                            session.last_at = now_utc;
                        }
                    }
                }
            }
            let closed = state.close_session();
            state.active = false;
            state.app = None;
            state.last_mono = Some(now_mono);
            state.last_utc = Some(now_utc);
            closed
        };
        if let Some(session) = closed_session {
            self.state.lock().pending_sessions.push(session);
        }
        self.flush(ctx, now_mono)?;
        let mut state = self.state.lock();
        state.pending.clear();
        state.pending_apps.clear();
        Ok(())
    }

    fn flush(&self, ctx: &ModuleContext, now_mono: u64) -> CoreResult<()> {
        let mut deltas = {
            let mut state = self.state.lock();
            let pending = std::mem::take(&mut state.pending);
            let mut grouped: HashMap<BucketKey, u64> = HashMap::new();
            for (key, millis) in pending {
                let whole = (millis / 1_000) * 1_000;
                if whole > 0 {
                    *grouped.entry(key.clone()).or_insert(0) += whole;
                }
                let remainder = millis % 1_000;
                if remainder > 0 {
                    state.pending.insert(key, remainder);
                }
            }
            let mut deltas = Vec::with_capacity(grouped.len());
            for (key, millis) in grouped {
                if let Some(app) = state.pending_apps.get(&key.app_id) {
                    deltas.push(HourlyDelta {
                        app: app.clone(),
                        date: key.date,
                        hour: key.hour,
                        seconds: millis / 1_000,
                    });
                }
            }
            std::mem::take(&mut deltas)
        };

        let mut sessions = std::mem::take(&mut self.state.lock().pending_sessions);

        if deltas.is_empty() && sessions.is_empty() {
            let mut state = self.state.lock();
            state.last_flush_mono = now_mono;
            return Ok(());
        }

        #[cfg(test)]
        self.write_attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let write_result = store::write_pending(ctx, &deltas, &sessions);
        if let Err(error) = write_result {
            // 写库失败时把整秒增量放回内存，下一次刷写重试，避免静默丢数据。
            let mut state = self.state.lock();
            for delta in deltas.drain(..) {
                let key = BucketKey {
                    date: delta.date,
                    hour: delta.hour,
                    app_id: delta.app.app_id,
                };
                *state.pending.entry(key).or_insert(0) += delta.seconds * 1_000;
            }
            state.pending_sessions.append(&mut sessions);
            return Err(error);
        }

        let mut state = self.state.lock();
        state.last_flush_mono = state.last_flush_mono.max(now_mono);
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/unit/tracker.rs"]
mod tests;
