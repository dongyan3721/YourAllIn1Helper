//! 剪贴板 IPC 适配与系统剪贴板写回。
use super::*;
/// 将文本写回操作系统剪贴板
fn write_text_to_system_clipboard(text: &str) -> CoreResult<()> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| CoreError::Internal(anyhow::anyhow!(error)))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|error| CoreError::Internal(anyhow::anyhow!(error)))
}

/// 将本地图片文件写回操作系统剪贴板
fn write_image_to_system_clipboard(image_path: &Path) -> CoreResult<()> {
    let img = image::open(image_path)
        .map_err(|e| CoreError::Internal(anyhow::anyhow!("打开剪贴板图片失败: {}", e)))?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let raw = rgba.into_raw();
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| CoreError::Internal(anyhow::anyhow!(e)))?;
    clipboard
        .set_image(arboard::ImageData {
            width: w as usize,
            height: h as usize,
            bytes: std::borrow::Cow::Borrowed(&raw),
        })
        .map_err(|e| CoreError::Internal(anyhow::anyhow!("写入系统剪贴板图片失败: {}", e)))?;
    Ok(())
}

impl DesktopHost {
    pub(super) fn dispatch_clipboard(&self, cmd: &str, args: Value) -> CoreResult<Value> {
        let ctx = self.kernel.context();
        match cmd {
            "clipboard_capture_text" => {
                let text = args.get("text").and_then(Value::as_str).unwrap_or("");
                let source_app = args.get("source_app_name").and_then(Value::as_str);
                let record = self.clipboard.capture_text(ctx, text, source_app)?;
                Ok(serde_json::to_value(record)?)
            }

            "clipboard_list_items" => {
                let keyword = args.get("keyword").and_then(Value::as_str);
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
                let items = self.clipboard.list_items(ctx, keyword, limit)?;
                Ok(serde_json::to_value(items)?)
            }

            "clipboard_paste_item" => {
                let id = args.get("id").and_then(Value::as_i64).unwrap_or(0);
                let as_plain = args
                    .get("as_plain_text")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                // 屏蔽剪贴板自捕获，避免写回操作系统剪贴板时再次被后台监听器重复记录
                ctx.event_bus.set_clipboard_suppression(true, "quick_paste");
                let record = self.clipboard.paste_item(ctx, id, as_plain);
                if let Ok(ref rec) = record {
                    if self.mock_hal.is_none() {
                        match rec.item_type {
                            mod_clipboard::ClipboardItemType::Text => {
                                if let Some(ref text) = rec.content_text {
                                    write_text_to_system_clipboard(text)?;
                                }
                            }
                            mod_clipboard::ClipboardItemType::Image => {
                                if let Some(ref rel_path) = rec.blob_rel_path {
                                    let full_path = ctx.app_data_dir.join(rel_path);
                                    if full_path.exists() {
                                        write_image_to_system_clipboard(&full_path)?;
                                    }
                                }
                            }
                        }
                    }
                }
                ctx.event_bus
                    .set_clipboard_suppression(false, "quick_paste_done");
                let record = record?;
                Ok(json!({
                    "pasted_record": record,
                    "paste_trigger_count": self.os_hal.input_simulator.paste_trigger_count(),
                    "active_window_count": ctx.window_pool.active_window_count(),
                }))
            }

            "clipboard_toggle_pin" => {
                let id = args.get("id").and_then(Value::as_i64).unwrap_or(0);
                let is_pinned = self.clipboard.toggle_pin(ctx, id)?;
                Ok(json!({ "id": id, "is_pinned": is_pinned }))
            }

            "clipboard_delete_item" => {
                let id = args.get("id").and_then(Value::as_i64).unwrap_or(0);
                let deleted = self.clipboard.delete_item(ctx, id)?;
                Ok(json!({ "id": id, "deleted": deleted }))
            }

            "clipboard_clear_unpinned" => {
                let deleted_count = self.clipboard.clear_unpinned(ctx)?;
                Ok(json!({ "deleted_count": deleted_count }))
            }

            "clipboard_set_suppression" => {
                let suppressed = args
                    .get("suppressed")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let reason = args
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("manual_ui_toggle");
                ctx.event_bus.set_clipboard_suppression(suppressed, reason);
                Ok(json!({
                    "clipboard_suppressed": ctx.event_bus.is_clipboard_suppressed()
                }))
            }

            _ => Err(CoreError::Config(format!("Unknown IPC command: {}", cmd))),
        }
    }
}
