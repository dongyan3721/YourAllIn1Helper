//! 模块启停、失败还原、配置持久化及并发执行隔离的回归测试。
use app_core::{AppKernelBuilder, AppModule, CoreError, CoreResult, ModuleContext, ShortcutSpec};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct Block {
    id: &'static str,
    starts: Arc<AtomicUsize>,
    stops: Arc<AtomicUsize>,
    fail: bool,
}
impl AppModule for Block {
    fn id(&self) -> &'static str {
        self.id
    }
    fn name(&self) -> &'static str {
        self.id
    }
    fn default_shortcuts(&self) -> Vec<ShortcutSpec> {
        vec![ShortcutSpec::new(
            self.id,
            format!("{}:run", self.id),
            "F1",
            "run",
        )]
    }
    fn on_init(&self, _: &ModuleContext) -> CoreResult<()> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            Err(CoreError::Config("init failed".into()))
        } else {
            Ok(())
        }
    }
    fn on_shutdown(&self, _: &ModuleContext) -> CoreResult<()> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn failed_enable_rolls_back_lifecycle_shortcuts_and_persisted_switch() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = app_core::AppConfig::default();
    config.modules.enabled.insert("broken".into(), false);
    app_core::ConfigMigrationPipeline::save_atomic(dir.path().join("config.json"), &config)
        .unwrap();
    let starts = Arc::new(AtomicUsize::new(0));
    let stops = Arc::new(AtomicUsize::new(0));
    let kernel = AppKernelBuilder::new(dir.path())
        .with_in_memory_db(true)
        .register_module(Block {
            id: "broken",
            starts: starts.clone(),
            stops: stops.clone(),
            fail: true,
        })
        .unwrap()
        .bootstrap()
        .unwrap();
    assert!(kernel.set_module_enabled("broken", true).is_err());
    assert!(!kernel.registry().is_enabled("broken"));
    assert!(!kernel.context().config.read().modules.is_enabled("broken"));
    assert!(kernel
        .context()
        .shortcut_registry
        .get_by_action("broken:run")
        .is_none());
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert_eq!(stops.load(Ordering::SeqCst), 1);
}

#[test]
fn failed_persistence_keeps_memory_and_module_state_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let starts = Arc::new(AtomicUsize::new(0));
    let stops = Arc::new(AtomicUsize::new(0));
    let kernel = AppKernelBuilder::new(dir.path())
        .with_in_memory_db(true)
        .register_module(Block {
            id: "block",
            starts: starts.clone(),
            stops: stops.clone(),
            fail: false,
        })
        .unwrap()
        .bootstrap()
        .unwrap();
    std::fs::remove_file(dir.path().join("config.json")).unwrap();
    std::fs::create_dir(dir.path().join("config.json")).unwrap();
    assert!(kernel.set_module_enabled("block", false).is_err());
    assert!(kernel.registry().is_enabled("block"));
    assert!(kernel.context().config.read().modules.is_enabled("block"));
    assert!(kernel
        .context()
        .shortcut_registry
        .get_by_action("block:run")
        .is_some());
    assert_eq!(starts.load(Ordering::SeqCst), 2);
    assert_eq!(stops.load(Ordering::SeqCst), 1);
}

#[test]
fn shortcut_conflicts_prevent_enable_before_starting_the_block() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = app_core::AppConfig::default();
    config.modules.enabled.insert("second".into(), false);
    app_core::ConfigMigrationPipeline::save_atomic(dir.path().join("config.json"), &config)
        .unwrap();
    let starts = Arc::new(AtomicUsize::new(0));
    let stops = Arc::new(AtomicUsize::new(0));
    let kernel = AppKernelBuilder::new(dir.path())
        .with_in_memory_db(true)
        .register_module(Block {
            id: "first",
            starts: starts.clone(),
            stops: stops.clone(),
            fail: false,
        })
        .unwrap()
        .register_module(Block {
            id: "second",
            starts: starts.clone(),
            stops: stops.clone(),
            fail: false,
        })
        .unwrap()
        .bootstrap()
        .unwrap();
    assert!(kernel.set_module_enabled("second", true).is_err());
    assert!(!kernel.registry().is_enabled("second"));
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    kernel.set_module_enabled("first", false).unwrap();
    kernel.set_module_enabled("second", true).unwrap();
    assert_eq!(
        kernel
            .context()
            .shortcut_registry
            .resolve_accelerator("F1")
            .unwrap()
            .0,
        "second"
    );
}

#[test]
fn disable_waits_for_inflight_work_then_rejects_new_work() {
    use std::{sync::mpsc, time::Duration};
    let dir = tempfile::tempdir().unwrap();
    let kernel = Arc::new(
        AppKernelBuilder::new(dir.path())
            .with_in_memory_db(true)
            .register_module(Block {
                id: "block",
                starts: Arc::default(),
                stops: Arc::default(),
                fail: false,
            })
            .unwrap()
            .bootstrap()
            .unwrap(),
    );
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let work_kernel = kernel.clone();
    let work = std::thread::spawn(move || {
        work_kernel
            .registry()
            .with_enabled("block", || {
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(())
            })
            .unwrap()
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    let disable_kernel = kernel.clone();
    let disable = std::thread::spawn(move || {
        disable_kernel.set_module_enabled("block", false).unwrap();
        done_tx.send(()).unwrap();
    });
    assert!(done_rx.recv_timeout(Duration::from_millis(50)).is_err());
    release_tx.send(()).unwrap();
    done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    work.join().unwrap();
    disable.join().unwrap();
    assert!(kernel.registry().with_enabled("block", || Ok(())).is_err());
}

struct FailedShutdown {
    running: Arc<std::sync::atomic::AtomicBool>,
    recover_fails: bool,
}

impl AppModule for FailedShutdown {
    fn id(&self) -> &'static str {
        "failed_shutdown"
    }
    fn name(&self) -> &'static str {
        "failed_shutdown"
    }
    fn default_shortcuts(&self) -> Vec<ShortcutSpec> {
        vec![ShortcutSpec::new(
            "failed_shutdown",
            "failed_shutdown:run",
            "F2",
            "run",
        )]
    }
    fn on_init(&self, _: &ModuleContext) -> CoreResult<()> {
        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn on_shutdown(&self, _: &ModuleContext) -> CoreResult<()> {
        self.running.store(false, Ordering::SeqCst);
        Err(CoreError::Config("flush failed".into()))
    }
    fn on_shutdown_failed(&self, _: &ModuleContext) -> CoreResult<()> {
        if self.recover_fails {
            Err(CoreError::Config("restart failed".into()))
        } else {
            self.running.store(true, Ordering::SeqCst);
            Ok(())
        }
    }
}

#[test]
fn failed_disable_only_restores_enabled_state_after_confirmed_recovery() {
    for recover_fails in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let running = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let kernel = AppKernelBuilder::new(dir.path())
            .with_in_memory_db(true)
            .register_module(FailedShutdown {
                running: running.clone(),
                recover_fails,
            })
            .unwrap()
            .bootstrap()
            .unwrap();
        let error = kernel
            .set_module_enabled("failed_shutdown", false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("flush failed"));
        assert_eq!(error.contains("restart failed"), recover_fails);
        assert_eq!(running.load(Ordering::SeqCst), !recover_fails);
        assert_eq!(
            kernel.registry().is_enabled("failed_shutdown"),
            !recover_fails
        );
        assert_eq!(
            kernel
                .context()
                .shortcut_registry
                .get_by_action("failed_shutdown:run")
                .is_some(),
            !recover_fails
        );
        assert_eq!(
            kernel
                .registry()
                .with_enabled("failed_shutdown", || Ok(()))
                .is_ok(),
            !recover_fails
        );
    }
}
