use super::*;
use app_core::{AppKernelBuilder, CloseFlushReport, TargetOs, WindowBackend, WindowSpec};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn test_sticky_note_lifecycle_and_zero_webview_when_closed() {
    let temp_dir = tempfile::tempdir().unwrap();
    let sticky = StickyNoteModule::new();

    // 1. 首次冷启动：0 个便笺打开 -> 0 个 WebView 实例
    let kernel = AppKernelBuilder::new(temp_dir.path())
        .with_target_os(TargetOs::MacOs)
        .register_module(sticky.clone())
        .unwrap()
        .bootstrap()
        .unwrap();

    let ctx = kernel.context();
    assert_eq!(ctx.window_pool.active_window_count(), 0);

    // 2. 创建便笺（包含 Markdown 与 LaTeX 公式）
    let note = sticky
        .create_note(
            ctx,
            Some("物理公式备忘"),
            Some(
                "质能方程：$E = mc^2$\n\n$$\\int_{-\\infty}^{\\infty} e^{-x^2} dx = \\sqrt{\\pi}$$",
            ),
            Some("sky"),
        )
        .unwrap();
    assert_eq!(ctx.window_pool.active_window_count(), 1);

    // 3. 更新窗口尺寸、状态与内容
    sticky
        .update_window_state(
            ctx,
            &note.id,
            StickyNoteWindowStateUpdate {
                pos_x: Some(400),
                pos_y: Some(260),
                width: 380,
                height: 340,
                is_pinned: false,
                opacity: 0.88,
            },
            None,
            None,
        )
        .unwrap();

    let updated = sticky
        .save_content(
            ctx,
            &note.id,
            Some("更新后的标题"),
            "# 标题\n- [x] 已完成",
            None,
            None,
        )
        .unwrap();
    assert_eq!(updated.title, "更新后的标题");
    assert!(!updated.is_pinned);
    assert_eq!(updated.width, 380);

    // 4. 导出 Markdown 格式验证
    let exported = sticky.export_markdown(ctx, &note.id).unwrap();
    assert!(exported.contains("# 更新后的标题"));
    assert!(exported.contains("- [x] 已完成"));

    // 5. 关闭便笺 -> 立即销毁 WebView 窗口释放内存
    sticky.close_note(ctx, &note.id).unwrap();
    assert_eq!(ctx.window_pool.active_window_count(), 0);
    kernel.shutdown().unwrap();
}

fn open_fixture() -> (tempfile::TempDir, app_core::AppKernel, StickyNoteModule) {
    let temp_dir = tempfile::tempdir().unwrap();
    let sticky = StickyNoteModule::new();
    let kernel = AppKernelBuilder::new(temp_dir.path())
        .with_target_os(TargetOs::MacOs)
        .register_module(sticky.clone())
        .unwrap()
        .bootstrap()
        .unwrap();
    (temp_dir, kernel, sticky)
}

fn window_state(opacity: f64, is_pinned: bool) -> StickyNoteWindowStateUpdate {
    StickyNoteWindowStateUpdate {
        pos_x: Some(240),
        pos_y: Some(180),
        width: 320,
        height: 300,
        is_pinned,
        opacity,
    }
}

#[test]
fn close_keeps_latest_text_theme_and_window_state_when_older_writes_arrive_later() {
    let (_dir, kernel, sticky) = open_fixture();
    let ctx = kernel.context();
    let note = sticky
        .create_note(ctx, Some("原标题"), Some("原正文"), Some("amber"))
        .unwrap();

    let fresh = sticky
        .save_content(ctx, &note.id, Some("最新标题"), "最新正文", Some(2), None)
        .unwrap();
    assert!(fresh.applied);
    let stale = sticky
        .save_content(ctx, &note.id, Some("旧标题"), "旧正文", Some(1), None)
        .unwrap();
    assert!(!stale.applied);
    assert_eq!(stale.title, "最新标题");
    assert_eq!(stale.content_md, "最新正文");
    // 同一序号是同一次请求的重试，必须真正写入。
    let retried = sticky
        .save_content(ctx, &note.id, Some("重试标题"), "重试正文", Some(2), None)
        .unwrap();
    assert!(retried.applied);
    assert_eq!(retried.content_md, "重试正文");

    let theme = sticky
        .update_theme(ctx, &note.id, "rose", Some(3), None)
        .unwrap();
    assert!(theme.applied);
    let stale_theme = sticky
        .update_theme(ctx, &note.id, "sky", Some(1), None)
        .unwrap();
    assert!(!stale_theme.applied);
    assert_eq!(stale_theme.theme_color, "rose");
    let window = sticky
        .update_window_state(ctx, &note.id, window_state(0.5, false), Some(5), None)
        .unwrap();
    assert!(window.applied);
    let stale_window = sticky
        .update_window_state(ctx, &note.id, window_state(0.75, true), Some(4), None)
        .unwrap();
    assert!(!stale_window.applied);
    assert!((stale_window.opacity - 0.5).abs() < f64::EPSILON);
    assert!(!stale_window.is_pinned);

    let missing = sticky.save_content(ctx, "missing-note", Some("x"), "y", Some(1), None);
    assert!(missing.is_err());

    sticky.close_note(ctx, &note.id).unwrap();
    assert_eq!(ctx.window_pool.active_window_count(), 0);
    let stored = sticky.get_note(ctx, &note.id).unwrap().unwrap();
    assert!(!stored.is_open);
    assert_eq!(stored.title, "重试标题");
    assert_eq!(stored.content_md, "重试正文");
    assert_eq!(stored.theme_color, "rose");
    assert!((stored.opacity - 0.5).abs() < f64::EPSILON);
    assert!(!stored.is_pinned);
    kernel.shutdown().unwrap();
}

#[test]
fn newer_write_seq_wins_when_older_and_newer_saves_race() {
    let (_dir, kernel, sticky) = open_fixture();
    let ctx = kernel.context().clone();
    let note = sticky
        .create_note(&ctx, Some("base"), Some("base"), Some("amber"))
        .unwrap();
    let id = note.id.clone();

    for round in 0..16 {
        let older_seq = round * 2 + 1;
        let newer_seq = round * 2 + 2;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let older = {
            let sticky = sticky.clone();
            let ctx = ctx.clone();
            let id = id.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                sticky
                    .save_content(
                        &ctx,
                        &id,
                        Some(&format!("old-title-{round}")),
                        &format!("old-body-{round}"),
                        Some(older_seq),
                        None,
                    )
                    .unwrap();
            })
        };
        let newer = {
            let sticky = sticky.clone();
            let ctx = ctx.clone();
            let id = id.clone();
            std::thread::spawn(move || {
                barrier.wait();
                sticky
                    .save_content(
                        &ctx,
                        &id,
                        Some(&format!("new-title-{round}")),
                        &format!("new-body-{round}"),
                        Some(newer_seq),
                        None,
                    )
                    .unwrap();
            })
        };
        older.join().unwrap();
        newer.join().unwrap();
        let stored = sticky.get_note(&ctx, &id).unwrap().unwrap();
        assert_eq!(stored.title, format!("new-title-{round}"));
        assert_eq!(stored.content_md, format!("new-body-{round}"));
    }

    for round in 0..8 {
        let older_seq = round * 2 + 1;
        let newer_seq = round * 2 + 2;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let older = {
            let sticky = sticky.clone();
            let ctx = ctx.clone();
            let id = id.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                sticky
                    .update_window_state(&ctx, &id, window_state(0.75, true), Some(older_seq), None)
                    .unwrap();
            })
        };
        let newer = {
            let sticky = sticky.clone();
            let ctx = ctx.clone();
            let id = id.clone();
            std::thread::spawn(move || {
                barrier.wait();
                sticky
                    .update_window_state(
                        &ctx,
                        &id,
                        window_state(0.25, false),
                        Some(newer_seq),
                        None,
                    )
                    .unwrap();
            })
        };
        older.join().unwrap();
        newer.join().unwrap();
        let stored = sticky.get_note(&ctx, &id).unwrap().unwrap();
        assert!((stored.opacity - 0.25).abs() < f64::EPSILON);
        assert!(!stored.is_pinned);
    }

    kernel.shutdown().unwrap();
}

/// 同一进程、同一便笺：关闭并重新打开后，新 WebView 的序号从 1 重新计。
/// 旧页面迟到的更大序号不得覆盖新会话已经保存的标题、正文、主题和窗口状态。
#[test]
fn reopened_note_in_the_same_process_saves_fresh_edits_and_ignores_the_closed_webview() {
    let (_dir, kernel, sticky) = open_fixture();
    let ctx = kernel.context().clone();
    let note = sticky
        .create_note(&ctx, Some("原标题"), Some("原正文"), Some("amber"))
        .unwrap();
    let id = note.id.clone();

    let first = sticky.begin_write_session(&ctx, &id).unwrap();
    sticky
        .save_content(
            &ctx,
            &id,
            Some("第一轮标题"),
            "第一轮正文",
            Some(5),
            Some(first),
        )
        .unwrap();
    sticky
        .update_theme(&ctx, &id, "rose", Some(4), Some(first))
        .unwrap();
    sticky
        .update_window_state(&ctx, &id, window_state(0.4, true), Some(6), Some(first))
        .unwrap();
    sticky.close_note(&ctx, &id).unwrap();
    assert_eq!(ctx.window_pool.active_window_count(), 0);

    let reopened = sticky.open_note(&ctx, &id).unwrap();
    assert!(reopened.is_open);
    assert_eq!(reopened.title, "第一轮标题");
    let second = sticky.begin_write_session(&ctx, &id).unwrap();
    assert_ne!(first, second);

    let saved = sticky
        .save_content(
            &ctx,
            &id,
            Some("第二轮标题"),
            "第二轮正文",
            Some(1),
            Some(second),
        )
        .unwrap();
    assert!(saved.applied);
    assert_eq!(saved.title, "第二轮标题");
    assert_eq!(saved.content_md, "第二轮正文");
    let theme = sticky
        .update_theme(&ctx, &id, "violet", Some(1), Some(second))
        .unwrap();
    assert!(theme.applied);
    let window = sticky
        .update_window_state(&ctx, &id, window_state(0.8, false), Some(1), Some(second))
        .unwrap();
    assert!(window.applied);

    let delayed = sticky
        .save_content(
            &ctx,
            &id,
            Some("过期标题"),
            "过期正文",
            Some(99),
            Some(first),
        )
        .unwrap();
    assert!(!delayed.applied);
    assert_eq!(delayed.title, "第二轮标题");
    assert_eq!(delayed.content_md, "第二轮正文");
    let delayed_theme = sticky
        .update_theme(&ctx, &id, "sky", Some(99), Some(first))
        .unwrap();
    assert!(!delayed_theme.applied);
    assert_eq!(delayed_theme.theme_color, "violet");
    let delayed_window = sticky
        .update_window_state(&ctx, &id, window_state(0.2, true), Some(99), Some(first))
        .unwrap();
    assert!(!delayed_window.applied);
    assert!((delayed_window.opacity - 0.8).abs() < f64::EPSILON);
    assert!(!delayed_window.is_pinned);
    let unscoped = sticky
        .save_content(&ctx, &id, Some("无会话标题"), "无会话正文", Some(100), None)
        .unwrap();
    assert!(!unscoped.applied);
    assert_eq!(unscoped.content_md, "第二轮正文");

    let stored = sticky.get_note(&ctx, &id).unwrap().unwrap();
    assert_eq!(stored.title, "第二轮标题");
    assert_eq!(stored.content_md, "第二轮正文");
    assert_eq!(stored.theme_color, "violet");
    assert!((stored.opacity - 0.8).abs() < f64::EPSILON);
    assert!(!stored.is_pinned);

    let retried = sticky
        .save_content(
            &ctx,
            &id,
            Some("重试第二轮"),
            "重试正文",
            Some(1),
            Some(second),
        )
        .unwrap();
    assert_eq!(retried.content_md, "重试正文");

    for round in 0..8 {
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let stale = {
            let sticky = sticky.clone();
            let ctx = ctx.clone();
            let id = id.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                sticky
                    .save_content(
                        &ctx,
                        &id,
                        Some(&format!("延迟旧标题-{round}")),
                        &format!("延迟旧正文-{round}"),
                        Some(50 + round),
                        Some(second),
                    )
                    .unwrap();
                sticky
                    .update_theme(&ctx, &id, "amber", Some(50 + round), Some(second))
                    .unwrap();
                sticky
                    .update_window_state(
                        &ctx,
                        &id,
                        window_state(0.2, true),
                        Some(50 + round),
                        Some(second),
                    )
                    .unwrap();
            })
        };
        let fresh = {
            let sticky = sticky.clone();
            let ctx = ctx.clone();
            let id = id.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let session = sticky.begin_write_session(&ctx, &id).unwrap();
                sticky
                    .save_content(
                        &ctx,
                        &id,
                        Some(&format!("并发新标题-{round}")),
                        &format!("并发新正文-{round}"),
                        Some(1),
                        Some(session),
                    )
                    .unwrap();
                sticky
                    .update_theme(&ctx, &id, "emerald", Some(1), Some(session))
                    .unwrap();
                sticky
                    .update_window_state(
                        &ctx,
                        &id,
                        window_state(0.6, false),
                        Some(1),
                        Some(session),
                    )
                    .unwrap();
            })
        };
        stale.join().unwrap();
        fresh.join().unwrap();
        let raced = sticky.get_note(&ctx, &id).unwrap().unwrap();
        assert_eq!(raced.title, format!("并发新标题-{round}"));
        assert_eq!(raced.content_md, format!("并发新正文-{round}"));
        assert_eq!(raced.theme_color, "emerald");
        assert!((raced.opacity - 0.6).abs() < f64::EPSILON);
        assert!(!raced.is_pinned);
    }

    let missing = sticky.begin_write_session(&ctx, "missing-note");
    assert!(missing.is_err());
    kernel.shutdown().unwrap();
}

/// 记录窗口生命周期，并可模拟页面在收到冲刷请求后写入最后一批内容。
struct FlushBackend {
    destroyed: std::sync::Mutex<Vec<String>>,
    flush_calls: std::sync::Mutex<Vec<String>>,
    cancelled: std::sync::Mutex<Vec<(String, u64)>>,
    outcome: std::sync::Mutex<CloseFlushOutcome>,
    next_request: std::sync::atomic::AtomicU64,
    on_flush: std::sync::Mutex<Option<Box<dyn FnMut(&str) + Send>>>,
}

impl FlushBackend {
    fn with_outcome(outcome: CloseFlushOutcome) -> Arc<Self> {
        Arc::new(Self {
            destroyed: std::sync::Mutex::new(Vec::new()),
            flush_calls: std::sync::Mutex::new(Vec::new()),
            cancelled: std::sync::Mutex::new(Vec::new()),
            outcome: std::sync::Mutex::new(outcome),
            next_request: std::sync::atomic::AtomicU64::new(0),
            on_flush: std::sync::Mutex::new(None),
        })
    }

    fn set_outcome(&self, outcome: CloseFlushOutcome) {
        *self.outcome.lock().unwrap_or_else(|err| err.into_inner()) = outcome;
    }

    fn set_on_flush(&self, callback: impl FnMut(&str) + Send + 'static) {
        *self.on_flush.lock().unwrap_or_else(|err| err.into_inner()) = Some(Box::new(callback));
    }

    fn destroyed(&self) -> Vec<String> {
        self.destroyed
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    fn flush_calls(&self) -> Vec<String> {
        self.flush_calls
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    fn cancelled(&self) -> Vec<(String, u64)> {
        self.cancelled
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }
}

impl WindowBackend for FlushBackend {
    fn create_or_focus_window(&self, _spec: &WindowSpec) -> CoreResult<()> {
        Ok(())
    }

    fn destroy_window(&self, label: &str) -> CoreResult<()> {
        self.destroyed
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(label.to_string());
        Ok(())
    }

    fn set_ignore_cursor_events(&self, _label: &str, _ignore: bool) -> CoreResult<()> {
        Ok(())
    }

    fn set_always_on_top(&self, _label: &str, _always_on_top: bool) -> CoreResult<()> {
        Ok(())
    }

    fn request_close_flush(&self, label: &str, _timeout: Duration) -> CoreResult<CloseFlushReport> {
        self.flush_calls
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(label.to_string());
        if let Some(callback) = self
            .on_flush
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .as_mut()
        {
            callback(label);
        }
        let request_id = self
            .next_request
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        Ok(CloseFlushReport {
            request_id,
            outcome: self
                .outcome
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone(),
        })
    }

    fn cancel_close_flush(&self, label: &str, request_id: u64) -> CoreResult<()> {
        self.cancelled
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push((label.to_string(), request_id));
        Ok(())
    }
}

fn fixture_with_backend(
    backend: Arc<FlushBackend>,
) -> (tempfile::TempDir, app_core::AppKernel, StickyNoteModule) {
    let temp_dir = tempfile::tempdir().unwrap();
    let sticky = StickyNoteModule::new();
    let kernel = AppKernelBuilder::new(temp_dir.path())
        .with_in_memory_db(true)
        .with_target_os(TargetOs::MacOs)
        .with_window_backend(backend)
        .register_module(sticky.clone())
        .unwrap()
        .bootstrap()
        .unwrap();
    (temp_dir, kernel, sticky)
}

#[test]
fn shutdown_flushes_the_page_and_only_then_destroys_the_window() {
    let backend = FlushBackend::with_outcome(CloseFlushOutcome::Saved);
    let (_dir, kernel, sticky) = fixture_with_backend(backend.clone());
    let ctx = kernel.context().clone();
    let note = sticky
        .create_note(&ctx, Some("old title"), Some("old body"), Some("amber"))
        .unwrap();
    let label = StickyNoteModule::window_label_for(&note.id);

    let flush_sticky = sticky.clone();
    let flush_ctx = ctx.clone();
    backend.set_on_flush(move |label| {
        let id = label.strip_prefix("sticky-note-").unwrap();
        flush_sticky
            .save_content(
                &flush_ctx,
                id,
                Some("page title"),
                "page body",
                Some(1),
                None,
            )
            .unwrap();
    });

    sticky.on_shutdown(&ctx).unwrap();
    assert_eq!(backend.flush_calls(), vec![label.clone()]);
    assert_eq!(backend.destroyed(), vec![label]);
    assert_eq!(ctx.window_pool.active_window_count(), 0);

    let stored = sticky.get_note(&ctx, &note.id).unwrap().unwrap();
    assert_eq!(stored.title, "page title");
    assert_eq!(stored.content_md, "page body");
    assert!(
        stored.is_open,
        "shutdown must keep the note open for restart restore"
    );
}

#[test]
fn failed_shutdown_flush_keeps_every_window_and_allows_a_retry() {
    let backend = FlushBackend::with_outcome(CloseFlushOutcome::Failed("disk full".into()));
    let (_dir, kernel, sticky) = fixture_with_backend(backend.clone());
    let ctx = kernel.context().clone();
    let note = sticky
        .create_note(&ctx, Some("kept title"), Some("kept body"), Some("amber"))
        .unwrap();
    let label = StickyNoteModule::window_label_for(&note.id);

    let error = sticky.on_shutdown(&ctx).unwrap_err().to_string();
    assert!(error.contains(&note.id), "{error}");
    assert!(error.contains("disk full"), "{error}");
    assert!(backend.destroyed().is_empty());
    assert_eq!(ctx.window_pool.active_window_count(), 1);
    let stored = sticky.get_note(&ctx, &note.id).unwrap().unwrap();
    assert_eq!(stored.title, "kept title");
    assert_eq!(stored.content_md, "kept body");
    assert!(stored.is_open);

    backend.set_outcome(CloseFlushOutcome::Saved);
    sticky.on_shutdown(&ctx).unwrap();
    assert_eq!(backend.destroyed(), vec![label]);
    assert_eq!(ctx.window_pool.active_window_count(), 0);
}

#[test]
fn timed_out_shutdown_flush_does_not_destroy_or_mark_the_note_closed() {
    let backend = FlushBackend::with_outcome(CloseFlushOutcome::TimedOut);
    let (_dir, kernel, sticky) = fixture_with_backend(backend.clone());
    let ctx = kernel.context().clone();
    let note = sticky
        .create_note(
            &ctx,
            Some("timeout title"),
            Some("timeout body"),
            Some("amber"),
        )
        .unwrap();

    let error = sticky.on_shutdown(&ctx).unwrap_err().to_string();
    assert!(error.contains("超时"), "{error}");
    assert!(backend.destroyed().is_empty());
    assert_eq!(ctx.window_pool.active_window_count(), 1);
    let stored = sticky.get_note(&ctx, &note.id).unwrap().unwrap();
    assert!(stored.is_open);
    assert_eq!(stored.content_md, "timeout body");
}

#[test]
fn kernel_shutdown_does_not_blind_destroy_windows_after_a_flush_failure() {
    let backend = FlushBackend::with_outcome(CloseFlushOutcome::Failed("busy".into()));
    let (_dir, kernel, sticky) = fixture_with_backend(backend.clone());
    let ctx = kernel.context();
    sticky
        .create_note(ctx, Some("keep me"), Some("unsaved body"), Some("amber"))
        .unwrap();

    let error = kernel.shutdown().unwrap_err().to_string();
    assert!(error.contains("busy"), "{error}");
    assert!(backend.destroyed().is_empty());
    assert_eq!(ctx.window_pool.active_window_count(), 1);
}

#[test]
fn failed_disable_keeps_sticky_enabled_and_windows_available_for_retry() {
    let backend = FlushBackend::with_outcome(CloseFlushOutcome::Failed("busy".into()));
    let (_dir, kernel, sticky) = fixture_with_backend(backend.clone());
    let ctx = kernel.context().clone();
    sticky
        .create_note(&ctx, Some("retry"), Some("body"), Some("amber"))
        .unwrap();

    let error = kernel
        .set_module_enabled("sticky_note", false)
        .unwrap_err()
        .to_string();
    assert!(error.contains("busy"), "{error}");
    assert!(kernel.registry().is_enabled("sticky_note"));
    assert_eq!(ctx.window_pool.active_window_count(), 1);
    assert!(backend.destroyed().is_empty());

    backend.set_outcome(CloseFlushOutcome::Saved);
    kernel.set_module_enabled("sticky_note", false).unwrap();
    assert!(!kernel.registry().is_enabled("sticky_note"));
    assert_eq!(ctx.window_pool.active_window_count(), 0);
}

#[test]
fn multi_note_flush_failure_releases_every_frozen_page_with_its_request_id() {
    let backend = FlushBackend::with_outcome(CloseFlushOutcome::Failed("disk full".into()));
    let (_dir, kernel, sticky) = fixture_with_backend(backend.clone());
    let ctx = kernel.context().clone();
    let first = sticky
        .create_note(&ctx, Some("first"), Some("one"), Some("amber"))
        .unwrap();
    let second = sticky
        .create_note(&ctx, Some("second"), Some("two"), Some("sky"))
        .unwrap();
    let first_label = StickyNoteModule::window_label_for(&first.id);
    let second_label = StickyNoteModule::window_label_for(&second.id);

    let error = sticky.on_shutdown(&ctx).unwrap_err().to_string();
    assert!(error.contains(&first.id), "{error}");
    assert!(error.contains(&second.id), "{error}");
    assert!(backend.destroyed().is_empty());
    assert_eq!(ctx.window_pool.active_window_count(), 2);

    let cancelled = backend.cancelled();
    assert_eq!(cancelled.len(), 2);
    assert!(cancelled
        .iter()
        .any(|(label, id)| *label == first_label && *id > 0));
    assert!(cancelled
        .iter()
        .any(|(label, id)| *label == second_label && *id > 0));
    assert_ne!(
        cancelled[0].1, cancelled[1].1,
        "each request has a unique id"
    );
}
