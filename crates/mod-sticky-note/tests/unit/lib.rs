use super::*;
use app_core::{AppKernelBuilder, TargetOs};

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
        )
        .unwrap();

    let updated = sticky
        .save_content(
            ctx,
            &note.id,
            Some("更新后的标题"),
            "# 标题\n- [x] 已完成",
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

    sticky
        .save_content(ctx, &note.id, Some("最新标题"), "最新正文", Some(2))
        .unwrap();
    let stale = sticky
        .save_content(ctx, &note.id, Some("旧标题"), "旧正文", Some(1))
        .unwrap();
    assert_eq!(stale.title, "最新标题");
    assert_eq!(stale.content_md, "最新正文");
    // 同一序号是同一次请求的重试，必须真正写入。
    let retried = sticky
        .save_content(ctx, &note.id, Some("重试标题"), "重试正文", Some(2))
        .unwrap();
    assert_eq!(retried.content_md, "重试正文");

    sticky.update_theme(ctx, &note.id, "rose", Some(3)).unwrap();
    sticky.update_theme(ctx, &note.id, "sky", Some(1)).unwrap();
    sticky
        .update_window_state(ctx, &note.id, window_state(0.5, false), Some(5))
        .unwrap();
    sticky
        .update_window_state(ctx, &note.id, window_state(0.75, true), Some(4))
        .unwrap();

    let missing = sticky.save_content(ctx, "missing-note", Some("x"), "y", Some(1));
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
                    .update_window_state(&ctx, &id, window_state(0.75, true), Some(older_seq))
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
                    .update_window_state(&ctx, &id, window_state(0.25, false), Some(newer_seq))
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
