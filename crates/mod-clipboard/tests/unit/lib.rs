use super::*;
use app_core::{AppKernelBuilder, TargetOs};
use os_hal::MockOsHal;

#[test]
fn test_clipboard_deduplication_lru_eviction_and_suppression() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mock_hal = MockOsHal::new();
    let clip_mod = ClipboardModule::new(Arc::new(mock_hal.clone()));

    let kernel = AppKernelBuilder::new(temp_dir.path())
        .with_in_memory_db(true)
        .with_target_os(TargetOs::MacOs)
        .register_module(ClipboardModule::new(Arc::new(mock_hal.clone())))
        .unwrap()
        .bootstrap()
        .unwrap();

    let ctx = kernel.context();

    // 调整容量上限为 3 条以测试 LRU 自动淘汰
    kernel
        .update_config(|cfg| {
            cfg.clipboard.max_history_items = 3;
        })
        .unwrap();

    // 1. 插入 3 条文本并固定第一条
    let r1 = clip_mod
        .capture_text(ctx, "First pinned item", Some("Notes"))
        .unwrap()
        .unwrap();
    assert!(clip_mod.toggle_pin(ctx, r1.id).unwrap());

    let r2 = clip_mod
        .capture_text(ctx, "Second item", Some("Browser"))
        .unwrap()
        .unwrap();
    let _r3 = clip_mod
        .capture_text(ctx, "Third item", Some("Editor"))
        .unwrap()
        .unwrap();
    let _r4 = clip_mod
        .capture_text(ctx, "Fourth item", Some("Terminal"))
        .unwrap()
        .unwrap();

    // 重新复制 "Second item"，验证排重且置顶
    let r2_again = clip_mod
        .capture_text(ctx, "Second item", Some("Browser"))
        .unwrap()
        .unwrap();
    assert_eq!(r2.id, r2_again.id);

    // 插入第 5 条，应当淘汰最旧的未固定项 ("Third item")，保留已固定的 r1 + 3条未固定 = 4条
    let _r5 = clip_mod
        .capture_text(ctx, "Fifth item", Some("Terminal"))
        .unwrap()
        .unwrap();

    let all = clip_mod.list_items(ctx, None, 50).unwrap();
    assert_eq!(all.len(), 4);
    assert_eq!(all[0].content_text.as_deref(), Some("First pinned item"));
    assert!(all[0].is_pinned);

    // 2. 测试防污染屏蔽守卫（如密码本复制或划词翻译取词）
    ctx.event_bus
        .set_clipboard_suppression(true, "password_vault_secret");
    let suppressed = clip_mod
        .capture_text(ctx, "SuperSecretPassword123!", None)
        .unwrap();
    assert!(suppressed.is_none());
    ctx.event_bus.set_clipboard_suppression(false, "done");

    // 3. 测试快捷键唤醒与粘贴销毁
    kernel.trigger_action("clipboard:toggle_popup").unwrap();
    assert_eq!(ctx.window_pool.active_window_count(), 1);

    clip_mod.paste_item(ctx, r2.id, true).unwrap();
    assert_eq!(ctx.window_pool.active_window_count(), 0);
    assert_eq!(mock_hal.paste_trigger_count(), 1);
}

#[test]
fn test_clipboard_image_capture_and_blob_storage() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mock_hal = MockOsHal::new();
    let clip_mod = ClipboardModule::new(Arc::new(mock_hal.clone()));

    let kernel = AppKernelBuilder::new(temp_dir.path())
        .with_in_memory_db(true)
        .with_target_os(TargetOs::MacOs)
        .register_module(ClipboardModule::new(Arc::new(mock_hal.clone())))
        .unwrap()
        .bootstrap()
        .unwrap();

    let ctx = kernel.context();

    // 构造一个 10x10 的红色 RGBA 像素图片
    let width = 10u32;
    let height = 10u32;
    let mut pixels = Vec::new();
    for _ in 0..(width * height) {
        pixels.extend_from_slice(&[255, 0, 0, 255]);
    }

    let rec = clip_mod
        .capture_image_rgba(ctx, width, height, &pixels, Some("ScreenshotApp"))
        .unwrap()
        .expect("图片条目应当成功捕获");

    assert_eq!(rec.item_type, ClipboardItemType::Image);
    assert!(rec.thumb_base64.is_some());
    assert!(rec.blob_rel_path.is_some());

    let blob_path = ctx.app_data_dir.join(rec.blob_rel_path.as_ref().unwrap());
    assert!(blob_path.exists());
}
