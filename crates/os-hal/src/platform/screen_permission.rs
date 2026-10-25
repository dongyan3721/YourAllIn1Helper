//! macOS 屏幕权限：被动查询不截图、不弹窗；只在用户主动开始截图时申请。
//!
//! 授权由 TCC 根据当前代码签名判定。窗口标题、图像颜色和自行缓存的布尔值均不能
//! 证明权限；特别是重新编译后的临时签名可能与系统设置中旧版本的授权不匹配。
use crate::{HalError, HalResult};
use objc2_core_graphics::{CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess};

fn denied() -> HalError {
    HalError::PermissionDenied {
        capability: "screen_recording",
        guidance: "请点击「开始截图」申请屏幕录制权限。若系统设置已允许却仍不可用，请完全退出本应用，在「隐私与安全性 → 屏幕与系统录音」移除旧条目，重新添加当前 App 并允许，再重新打开。请勿在授权后替换 App 文件。".into(),
    }
}

/// 每次读取系统当前结果；撤销后立即生效，不永久缓存曾经授权的状态。
pub(super) fn check() -> HalResult<()> {
    if CGPreflightScreenCaptureAccess() {
        Ok(())
    } else {
        Err(denied())
    }
}

/// CoreGraphics 的 C 接口由截图工作线程调用，避免系统授权对话框阻塞 AppKit 事件循环。
/// Request 返回值不覆盖后续状态，防止系统仍要求重启时直接抓到仅壁纸的画面。
pub(super) fn request() -> HalResult<()> {
    if check().is_ok() {
        return Ok(());
    }
    let _ = CGRequestScreenCaptureAccess();
    check()
}
