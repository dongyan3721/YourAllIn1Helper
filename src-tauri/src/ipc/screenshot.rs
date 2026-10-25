//! 增强截图 IPC 适配。
//!
//! JSON 路由只承载小数据；冻结帧、贴图 PNG 与标注层经 `screenshot_binary.rs` 的二进制命令传输，
//! 这些入口同样经 `with_enabled` 进入，停用时拒绝。
use super::*;
use app_core::SelectionBox;
use mod_screenshot::{
    config::{merge_settings, ScreenshotSettings},
    export::ExportAction,
    ExportOutcome, MODULE_ID,
};
use std::sync::Arc;

fn str_arg<'a>(args: &'a Value, key: &str) -> CoreResult<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| CoreError::Config(format!("缺少参数 {}", key)))
}

impl DesktopHost {
    pub(super) fn dispatch_screenshot(&self, cmd: &str, args: Value) -> CoreResult<Value> {
        let ctx = self.kernel.context();
        let shot = &self.screenshot;
        match cmd {
            "screenshot_get_status" => {
                let mut status = serde_json::to_value(shot.status(ctx))?;
                // 系统级快捷键注册失败只有宿主知道，单独附加，避免界面误报 F1 可用。
                status["native_shortcut_error"] = self
                    .native_shortcut_error(mod_screenshot::ACTION_CAPTURE)
                    .map(Value::String)
                    .unwrap_or(Value::Null);
                Ok(status)
            }
            "screenshot_start_capture" => {
                let monitor_id = args
                    .get("monitor_id")
                    .and_then(Value::as_u64)
                    .map(|id| id as u32);
                let hide_dashboard = args
                    .get("hide_dashboard")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let intent = match args.get("intent").and_then(Value::as_str) {
                    Some("ocr") => mod_screenshot::ocr::CaptureIntent::Ocr,
                    _ => mod_screenshot::ocr::CaptureIntent::Capture,
                };
                Ok(serde_json::to_value(shot.start_capture_with_intent(
                    ctx,
                    monitor_id,
                    hide_dashboard,
                    intent,
                )?)?)
            }
            "screenshot_ocr" => {
                let id = str_arg(&args, "session_id")?;
                let selection: SelectionBox =
                    serde_json::from_value(args.get("selection").cloned().unwrap_or(Value::Null))
                        .map_err(|e| CoreError::Config(format!("选区参数无效：{}", e)))?;
                Ok(serde_json::to_value(shot.recognize_text(id, selection)?)?)
            }
            "screenshot_copy_text" => {
                shot.copy_text(str_arg(&args, "text")?)?;
                Ok(json!({ "copied": true }))
            }
            "screenshot_get_session" => {
                let id = args.get("session_id").and_then(Value::as_str);
                Ok(serde_json::to_value(shot.session_info(ctx, id)?)?)
            }
            "screenshot_overlay_ready" => {
                shot.overlay_ready(ctx, str_arg(&args, "session_id")?)?;
                Ok(json!({ "shown": true }))
            }
            "screenshot_resolve_region" => {
                let id = str_arg(&args, "session_id")?;
                let x = args.get("x").and_then(Value::as_i64).unwrap_or(-1) as i32;
                let y = args.get("y").and_then(Value::as_i64).unwrap_or(-1) as i32;
                let steps = args.get("level_steps").and_then(Value::as_u64).unwrap_or(0) as usize;
                Ok(serde_json::to_value(shot.resolve_region(id, x, y, steps)?)?)
            }
            "screenshot_cancel" => {
                let id = args.get("session_id").and_then(Value::as_str);
                Ok(json!({ "cancelled": shot.cancel_capture(ctx, id)? }))
            }
            "screenshot_copy_color" => {
                shot.copy_color(str_arg(&args, "text")?)?;
                Ok(json!({ "copied": true }))
            }
            "screenshot_list_monitors" => Ok(serde_json::to_value(shot.list_monitors()?)?),
            "screenshot_list_pins" => Ok(serde_json::to_value(shot.list_pins())?),
            "screenshot_pin_info" => Ok(serde_json::to_value(
                shot.pin_summary(str_arg(&args, "pin_id")?)?,
            )?),
            "screenshot_close_pin" => Ok(json!({
                "closed": shot.close_pin(ctx, str_arg(&args, "pin_id")?)?
            })),
            "screenshot_close_all_pins" => Ok(json!({ "closed": shot.close_all_pins(ctx)? })),
            "screenshot_pin_copy" => {
                shot.copy_pin(str_arg(&args, "pin_id")?)?;
                Ok(json!({ "copied": true }))
            }
            "screenshot_pin_save" => Ok(json!({
                "saved_path": shot.save_pin(ctx, str_arg(&args, "pin_id")?)?
            })),
            "screenshot_get_config" => Ok(json!({
                "settings": shot.settings(ctx),
                "overlay": ctx.config.read().screenshot.clone(),
            })),
            "screenshot_update_settings" => self.update_screenshot_settings(&args),
            // ── M3: 滚动长截图 ──
            "screenshot_scroll_start" => {
                let session_id = str_arg(&args, "session_id")?;
                let sel_val = args
                    .get("selection")
                    .ok_or_else(|| CoreError::Config("缺少 selection 参数".into()))?;
                let selection: SelectionBox = serde_json::from_value(sel_val.clone())
                    .map_err(|e| CoreError::Config(format!("选区参数格式无效: {}", e)))?;
                Ok(serde_json::to_value(
                    shot.start_scroll(ctx, session_id, selection)?,
                )?)
            }
            "screenshot_scroll_stop" => Ok(serde_json::to_value(shot.stop_scroll(ctx)?)?),
            "screenshot_scroll_cancel" => Ok(json!({ "cancelled": shot.cancel_scroll(ctx)? })),
            "screenshot_scroll_status" => Ok(serde_json::to_value(shot.scroll_status())?),
            "screenshot_scroll_set_auto" => {
                let enabled = args
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                shot.set_scroll_auto(enabled)?;
                Ok(json!({ "ok": true }))
            }
            "screenshot_scroll_info" => Ok(serde_json::to_value(shot.stitched_image_info()?)?),
            "screenshot_scroll_export" => {
                let action_str = str_arg(&args, "action")?;
                let action = match action_str {
                    "save" => ExportAction::Save,
                    "copy" => ExportAction::Copy,
                    "pin" => ExportAction::Pin,
                    other => return Err(CoreError::Config(format!("未知导出操作: {}", other))),
                };
                Ok(serde_json::to_value(shot.export_scroll(ctx, action)?)?)
            }
            // ── M3: GIF 录制 ──
            "screenshot_gif_start" => {
                let session_id = str_arg(&args, "session_id")?;
                let sel_val = args
                    .get("selection")
                    .ok_or_else(|| CoreError::Config("缺少 selection 参数".into()))?;
                let selection: SelectionBox = serde_json::from_value(sel_val.clone())
                    .map_err(|e| CoreError::Config(format!("选区参数格式无效: {}", e)))?;
                let mut config = mod_screenshot::gif::GifRecordingConfig::default();
                if let Some(preset_str) = args.get("preset").and_then(Value::as_str) {
                    config.resolution_preset = match preset_str {
                        "1080p" => mod_screenshot::gif::GifResolutionPreset::P1080,
                        "720p" => mod_screenshot::gif::GifResolutionPreset::P720,
                        "480p" => mod_screenshot::gif::GifResolutionPreset::P480,
                        "custom" => {
                            let cw = args
                                .get("custom_width")
                                .and_then(Value::as_u64)
                                .unwrap_or(400) as u32;
                            let ch = args
                                .get("custom_height")
                                .and_then(Value::as_u64)
                                .unwrap_or(300) as u32;
                            mod_screenshot::gif::GifResolutionPreset::Custom {
                                width: cw,
                                height: ch,
                            }
                        }
                        _ => mod_screenshot::gif::GifResolutionPreset::Original,
                    };
                }
                if let Some(fps_val) = args.get("fps").and_then(Value::as_u64) {
                    config.framerate = mod_screenshot::gif::GifFramerate::from_u32(fps_val as u32);
                }
                if let Some(max_dur) = args.get("max_duration_secs").and_then(Value::as_u64) {
                    config.max_duration_secs = max_dur;
                }
                let overlay_png = args
                    .get("overlay_png_base64")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(|value| {
                        base64::Engine::decode(
                            &base64::engine::general_purpose::STANDARD,
                            value,
                        )
                        .map_err(|e| CoreError::Config(format!("标注层数据无效: {}", e)))
                    })
                    .transpose()?;
                Ok(serde_json::to_value(shot.start_gif(
                    ctx,
                    session_id,
                    selection,
                    config,
                    overlay_png,
                )?)?)
            }
            "screenshot_gif_stop" => Ok(serde_json::to_value(shot.stop_gif(ctx)?)?),
            "screenshot_gif_confirm" => Ok(serde_json::to_value(shot.confirm_gif(ctx)?)?),
            "screenshot_gif_copy" => {
                shot.copy_gif()?;
                Ok(json!({ "copied": true }))
            }
            "screenshot_gif_cancel" => Ok(json!({ "cancelled": shot.cancel_gif(ctx)? })),
            "screenshot_gif_status" => Ok(serde_json::to_value(shot.gif_status())?),
            _ => Err(CoreError::Config(format!(
                "Unknown screenshot command: {}",
                cmd
            ))),
        }
    }

    /// 合并保存模块配置与核心层截图开关；一次原子写入，失败不发布新配置。
    fn update_screenshot_settings(&self, args: &Value) -> CoreResult<Value> {
        let ctx = self.kernel.context();
        let current = self.screenshot.settings(ctx);
        let merged: ScreenshotSettings = match args.get("settings") {
            Some(patch) => merge_settings(&current, patch).map_err(CoreError::Config)?,
            None => current,
        };
        let overlay = args.get("overlay").cloned().unwrap_or(Value::Null);
        let flag = |key: &str| overlay.get(key).and_then(Value::as_bool);
        let (magnifier, smart, wheel, manual) = (
            flag("show_magnifier"),
            flag("enable_smart_region_detection"),
            flag("smart_region_wheel_expand"),
            flag("enable_manual_box_selection"),
        );
        let stored = serde_json::to_value(&merged)?;
        self.kernel.update_config(|config| {
            config
                .modules
                .settings
                .insert(MODULE_ID.to_string(), stored);
            if let Some(v) = magnifier {
                config.screenshot.show_magnifier = v;
            }
            if let Some(v) = smart {
                config.screenshot.enable_smart_region_detection = v;
            }
            if let Some(v) = wheel {
                config.screenshot.smart_region_wheel_expand = v;
            }
            if let Some(v) = manual {
                config.screenshot.enable_manual_box_selection = v;
            }
        })?;
        Ok(json!({
            "settings": self.screenshot.settings(ctx),
            "overlay": ctx.config.read().screenshot.clone(),
        }))
    }

    /// 冻结帧原始 RGBA（二进制 IPC）。
    pub fn screenshot_frame_bytes(&self, session_id: &str) -> CoreResult<Vec<u8>> {
        self.kernel
            .registry()
            .with_enabled(MODULE_ID, || self.screenshot.frame_bytes(session_id))
    }

    /// 贴图 PNG（二进制 IPC）。
    pub fn screenshot_pin_png(&self, pin_id: &str) -> CoreResult<Arc<Vec<u8>>> {
        self.kernel
            .registry()
            .with_enabled(MODULE_ID, || self.screenshot.pin_png(pin_id))
    }

    /// 导出选区：标注层 PNG 作为原始请求体传入（可为空）。
    pub fn screenshot_export(
        &self,
        session_id: &str,
        selection: SelectionBox,
        action: ExportAction,
        overlay_png: Option<&[u8]>,
    ) -> CoreResult<ExportOutcome> {
        self.kernel.registry().with_enabled(MODULE_ID, || {
            self.screenshot.export(
                self.kernel.context(),
                session_id,
                selection,
                overlay_png,
                action,
            )
        })
    }

    /// 长截图拼接产物原始 RGBA（二进制 IPC）。
    pub fn screenshot_scroll_image_bytes(&self) -> CoreResult<Vec<u8>> {
        self.kernel
            .registry()
            .with_enabled(MODULE_ID, || self.screenshot.stitched_image_bytes())
    }

    /// 长截图进行中的缩略图 PNG。
    pub fn screenshot_scroll_preview_png(&self) -> CoreResult<Vec<u8>> {
        self.kernel
            .registry()
            .with_enabled(MODULE_ID, || self.screenshot.scroll_preview_png())
    }

    /// 待确认 GIF 的原始字节。
    pub fn screenshot_gif_bytes(&self) -> CoreResult<Vec<u8>> {
        self.kernel
            .registry()
            .with_enabled(MODULE_ID, || self.screenshot.gif_bytes())
    }
}
