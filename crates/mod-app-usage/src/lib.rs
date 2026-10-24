//! `mod-app-usage` —— 子功能 05：应用使用时长统计。
//!
//! 通过注入的 `ActiveAppWatcher` 读取真实前台应用、键鼠空闲与锁屏/息屏状态；
//! 使用单调时钟累计时长，按 UTC 小时切段后映射到本地日期/小时。
//! 焦点切换、每满刷写间隔、停用和退出都会把内存增量写入 SQLite。
//!
//! 隐私边界：只记录应用标识、名称、可选窗口标题和秒数，不采集按键内容，
//! 不开启任何日常应用的后台统计；窗口标题是否保留由模块配置控制。

pub mod config;
pub mod report;
pub mod store;
pub mod tracker;

use crate::config::{AppUsageConfig, MODULE_ID};
use crate::report::{
    AppUsageCapabilities, AppUsageStatus, HourlyBreakdown, TrackedApp, UsageOverview,
};
use crate::tracker::{SystemUsageClock, Tracker, UsageClock, SAMPLE_INTERVAL_MS};
use app_core::{AppModule, CoreError, CoreResult, ModuleContext, ModuleMigration};
use os_hal::ActiveAppWatcher;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::{info, warn};

pub struct AppUsageModule {
    tracker: Arc<Tracker>,
    worker: Mutex<Option<JoinHandle<()>>>,
    stop: Arc<AtomicBool>,
    running: AtomicBool,
}

impl AppUsageModule {
    /// 生产/集成入口：注入真实或显式的系统适配器，使用系统时钟。
    pub fn new(watcher: Arc<dyn ActiveAppWatcher>) -> Self {
        Self::with_clock(watcher, Arc::new(SystemUsageClock::new()))
    }

    /// 测试入口：注入可控时间源，验证切段、休眠跳变与重启行为。
    pub fn with_clock(watcher: Arc<dyn ActiveAppWatcher>, clock: Arc<dyn UsageClock>) -> Self {
        Self {
            tracker: Arc::new(Tracker::new(watcher, clock)),
            worker: Mutex::new(None),
            stop: Arc::new(AtomicBool::new(false)),
            running: AtomicBool::new(false),
        }
    }

    pub fn is_tracking(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn config(&self) -> AppUsageConfig {
        self.tracker.config()
    }

    /// 应用配置（会做范围夹紧）；用于测试或已在外部完成持久化的调用点。
    pub fn apply_config(&self, config: AppUsageConfig) {
        self.tracker.apply_config(config);
    }

    pub fn capabilities(&self) -> AppUsageCapabilities {
        self.tracker.capabilities()
    }

    pub fn status(&self) -> AppUsageStatus {
        AppUsageStatus {
            tracking: self.is_tracking(),
            sample_interval_ms: SAMPLE_INTERVAL_MS,
            capabilities: self.capabilities(),
            config: self.config(),
        }
    }

    /// 立即采样一次；后台线程使用同一入口，测试无需真实等待。
    pub fn sample_now(&self, ctx: &ModuleContext) -> CoreResult<()> {
        self.tracker.sample(ctx)
    }

    /// 立即把内存中的整秒增量写入 SQLite，不停止采样线程。
    pub fn flush_pending(&self, ctx: &ModuleContext) -> CoreResult<()> {
        self.tracker.flush_now(ctx)
    }

    pub fn overview(
        &self,
        ctx: &ModuleContext,
        start_date: &str,
        end_date: &str,
    ) -> CoreResult<UsageOverview> {
        store::overview(ctx, start_date, end_date)
    }

    pub fn hourly(&self, ctx: &ModuleContext, date: &str) -> CoreResult<HourlyBreakdown> {
        store::hourly(ctx, date)
    }

    pub fn list_tracked_apps(&self, ctx: &ModuleContext) -> CoreResult<Vec<TrackedApp>> {
        store::list_tracked_apps(ctx)
    }

    /// 更新逐应用分类与忽略状态；忽略后下一次采样立即停止累计。
    pub fn set_app_category(
        &self,
        ctx: &ModuleContext,
        app_id: &str,
        category: &str,
        ignored: bool,
    ) -> CoreResult<TrackedApp> {
        let app = store::set_app_category(ctx, app_id, category, ignored)?;
        self.tracker.set_ignored(app_id.trim(), ignored);
        Ok(app)
    }

    pub fn today_local(&self) -> String {
        store::today_local()
    }

    /// 默认报表范围：含今天在内的最近 7 个本地日期。
    pub fn default_range(&self) -> (String, String) {
        let end = chrono::Local::now().date_naive();
        let start = end - chrono::Duration::days(6);
        (
            start.format("%Y-%m-%d").to_string(),
            end.format("%Y-%m-%d").to_string(),
        )
    }

    /// 从共享配置重新读取模块设置并应用到计时器（开关即时生效，无需重启采样线程）。
    pub fn reload_config(&self, ctx: &ModuleContext) -> CoreResult<AppUsageConfig> {
        let config = ctx
            .module_settings::<AppUsageConfig>(MODULE_ID)
            .unwrap_or_default()
            .normalized();
        self.tracker.apply_config(config.clone());
        Ok(config)
    }

    fn start_worker(&self, ctx: &ModuleContext) -> CoreResult<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.stop.store(false, Ordering::SeqCst);
        let tracker = Arc::clone(&self.tracker);
        let stop = Arc::clone(&self.stop);
        let ctx = ctx.clone();
        let handle = thread::Builder::new()
            .name("app-usage-sampler".to_string())
            .spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(SAMPLE_INTERVAL_MS));
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Err(error) = tracker.sample(&ctx) {
                        warn!(%error, "App usage sample failed");
                    }
                }
            });
        match handle {
            Ok(handle) => {
                *self.worker.lock() = Some(handle);
                info!(
                    interval_ms = SAMPLE_INTERVAL_MS,
                    "Started app usage sampler"
                );
                Ok(())
            }
            Err(error) => {
                self.running.store(false, Ordering::SeqCst);
                Err(CoreError::Io(error))
            }
        }
    }

    fn stop_worker(&self, ctx: &ModuleContext) -> CoreResult<()> {
        let was_running = self.running.swap(false, Ordering::SeqCst);
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.worker.lock().take() {
            let _ = handle.join();
        }
        if !was_running {
            return Ok(());
        }
        let result = self.tracker.finish(ctx);
        if let Err(error) = &result {
            warn!(%error, "Failed to flush app usage on stop");
        }
        info!("Stopped app usage sampler and flushed pending usage");
        result
    }
}

impl AppModule for AppUsageModule {
    fn id(&self) -> &'static str {
        MODULE_ID
    }

    fn name(&self) -> &'static str {
        "应用使用时长统计"
    }

    fn migrations(&self) -> Vec<ModuleMigration> {
        store::migrations()
    }

    fn on_init(&self, ctx: &ModuleContext) -> CoreResult<()> {
        let _ = self.reload_config(ctx);
        match store::load_ignored(ctx) {
            Ok(ignored) => {
                for app_id in ignored {
                    self.tracker.set_ignored(&app_id, true);
                }
            }
            Err(error) => warn!(%error, "Failed to load ignored app list"),
        }
        self.tracker.begin();
        self.start_worker(ctx)
    }

    fn on_shutdown(&self, ctx: &ModuleContext) -> CoreResult<()> {
        self.stop_worker(ctx)
    }
}

#[cfg(test)]
#[path = "../tests/unit/lib.rs"]
mod tests;
