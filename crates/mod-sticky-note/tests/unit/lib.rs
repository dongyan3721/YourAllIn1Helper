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
        )
        .unwrap();

    let updated = sticky
        .save_content(ctx, &note.id, Some("更新后的标题"), "# 标题\n- [x] 已完成")
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
