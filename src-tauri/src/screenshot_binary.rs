//! 截图大数据的二进制 IPC 命令。
//!
//! - `screenshot_frame`：冻结帧 RGBA 以 `tauri::ipc::Response` 原始字节返回，前端得到 `ArrayBuffer`，
//!   直接构造 `ImageData`；不经 JSON/base64。跨进程仍有一次复制，不属于零拷贝。
//! - `screenshot_pin_image`：贴图 PNG 原始字节。
//! - `screenshot_export`：标注层 PNG 作为原始请求体，导出参数放在请求头 JSON 中。
//!
//! 三个命令都是 async，耗时的复制/编码放到阻塞线程池，不占用 AppKit 主线程；
//! 进入模块前经 `with_enabled` 检查，停用后直接拒绝。
use crate::{DesktopHost, IpcResponse};
use app_core::SelectionBox;
use mod_screenshot::export::ExportAction;
use serde::Deserialize;
use std::sync::Arc;
use tauri::{ipc::InvokeBody, AppHandle, Emitter, Manager};

/// 导出参数请求头。
const EXPORT_HEADER: &str = "x-aio-screenshot-export";

#[derive(Debug, Deserialize)]
struct ExportMeta {
    session_id: String,
    selection: SelectionBox,
    action: ExportAction,
}

fn host(app: &AppHandle) -> Arc<DesktopHost> {
    app.state::<Arc<DesktopHost>>().inner().clone()
}

#[tauri::command]
pub(crate) async fn screenshot_frame(
    app: AppHandle,
    session_id: String,
) -> Result<tauri::ipc::Response, String> {
    let host = host(&app);
    tauri::async_runtime::spawn_blocking(move || host.screenshot_frame_bytes(&session_id))
        .await
        .map_err(|e| e.to_string())?
        .map(tauri::ipc::Response::new)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn screenshot_pin_image(
    app: AppHandle,
    pin_id: String,
) -> Result<tauri::ipc::Response, String> {
    let host = host(&app);
    tauri::async_runtime::spawn_blocking(move || host.screenshot_pin_png(&pin_id))
        .await
        .map_err(|e| e.to_string())?
        .map(|png| tauri::ipc::Response::new(png.as_ref().clone()))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn screenshot_export(
    app: AppHandle,
    request: tauri::ipc::Request<'_>,
) -> Result<IpcResponse, String> {
    let meta = request
        .headers()
        .get(EXPORT_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "缺少截图导出参数".to_string())
        .and_then(|raw| {
            serde_json::from_str::<ExportMeta>(raw).map_err(|e| format!("导出参数无效：{}", e))
        });
    let meta = match meta {
        Ok(meta) => meta,
        Err(error) => return Ok(IpcResponse::err(error)),
    };
    let overlay = match request.body() {
        InvokeBody::Raw(bytes) if !bytes.is_empty() => Some(bytes.clone()),
        _ => None,
    };
    let host = host(&app);
    let response = tauri::async_runtime::spawn_blocking(move || {
        match host.screenshot_export(
            &meta.session_id,
            meta.selection,
            meta.action,
            overlay.as_deref(),
        ) {
            Ok(outcome) => IpcResponse::success(outcome),
            Err(error) => IpcResponse::err(error.to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    // 导出会结束会话或新增贴图，通知主面板刷新。
    let _ = app.emit("kernel://updated", serde_json::Value::Null);
    Ok(response)
}

#[tauri::command]
pub(crate) async fn screenshot_scroll_image(
    app: AppHandle,
) -> Result<tauri::ipc::Response, String> {
    let host = host(&app);
    tauri::async_runtime::spawn_blocking(move || host.screenshot_scroll_image_bytes())
        .await
        .map_err(|e| e.to_string())?
        .map(tauri::ipc::Response::new)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn screenshot_scroll_preview(
    app: AppHandle,
) -> Result<tauri::ipc::Response, String> {
    let host = host(&app);
    tauri::async_runtime::spawn_blocking(move || host.screenshot_scroll_preview_png())
        .await
        .map_err(|e| e.to_string())?
        .map(tauri::ipc::Response::new)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn screenshot_gif_image(
    app: AppHandle,
) -> Result<tauri::ipc::Response, String> {
    let host = host(&app);
    tauri::async_runtime::spawn_blocking(move || host.screenshot_gif_bytes())
        .await
        .map_err(|e| e.to_string())?
        .map(tauri::ipc::Response::new)
        .map_err(|e| e.to_string())
}
