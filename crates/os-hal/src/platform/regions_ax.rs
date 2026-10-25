//! L2 无障碍控件层级（macOS AXUIElement）。
//!
//! 截图遮罩是本应用最前方的全屏窗口，系统级 AX 命中只会得到遮罩自身。因此先用窗口列表
//! （已排除本进程）找到光标处最前方的外部窗口，再对该应用的 AX 树做命中，向上收集父级容器。
//! 需要「辅助功能」权限；未授权时只报告 `PermissionDenied`，不调用会弹窗的授权接口。
use super::NativeOsHal;
use crate::{HalError, HalResult};
use app_core::{DetectedRegionRect, RegionHierarchyLevel};
use objc2_core_foundation::CFString;
use std::ffi::c_void;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> *mut c_void;
    fn AXUIElementCopyElementAtPosition(
        application: *mut c_void,
        x: f32,
        y: f32,
        element: *mut *mut c_void,
    ) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: *mut c_void,
        attribute: *const c_void,
        value: *mut *mut c_void,
    ) -> i32;
    fn AXUIElementSetMessagingTimeout(element: *mut c_void, timeout_seconds: f32) -> i32;
    fn AXValueGetValue(value: *const c_void, the_type: u32, value_ptr: *mut c_void) -> bool;
    fn CFRelease(cf: *const c_void);
    fn CFGetTypeID(cf: *const c_void) -> usize;
    fn CFStringGetTypeID() -> usize;
}

const AX_SUCCESS: i32 = 0;
const AX_VALUE_CG_POINT: u32 = 1;
const AX_VALUE_CG_SIZE: u32 = 2;
/// 层级链最大深度，防止异常 AX 树造成长时间遍历。
const MAX_DEPTH: usize = 16;
/// 单次 AX 消息超时：目标应用无响应时快速放弃，不拖慢遮罩交互。
const MESSAGING_TIMEOUT_SECS: f32 = 0.15;
/// 叶子 ID 起点（会话内再重新编号）。
const AX_REGION_BASE_ID: u32 = 50_000;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Size {
    width: f64,
    height: f64,
}

/// 自动释放的 CF 对象指针。
struct CfOwned(*mut c_void);

impl Drop for CfOwned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: 指针来自 Copy/Create 规则返回的 +1 引用。
            unsafe { CFRelease(self.0) };
        }
    }
}

fn attribute(element: *mut c_void, name: &str) -> Option<CfOwned> {
    let key = CFString::from_str(name);
    let mut value: *mut c_void = std::ptr::null_mut();
    // SAFETY: element 为存活的 AXUIElement；key 在调用期间存活。
    let status = unsafe {
        AXUIElementCopyAttributeValue(
            element,
            &*key as *const CFString as *const c_void,
            &mut value,
        )
    };
    (status == AX_SUCCESS && !value.is_null()).then_some(CfOwned(value))
}

fn string_attribute(element: *mut c_void, name: &str) -> Option<String> {
    let value = attribute(element, name)?;
    // SAFETY: 先校验类型再按 CFString 读取。
    unsafe {
        if CFGetTypeID(value.0) != CFStringGetTypeID() {
            return None;
        }
        Some((*(value.0 as *const CFString)).to_string())
    }
}

fn frame_of(element: *mut c_void) -> Option<(f64, f64, f64, f64)> {
    let position = attribute(element, "AXPosition")?;
    let size = attribute(element, "AXSize")?;
    let mut point = Point::default();
    let mut extent = Size::default();
    // SAFETY: AXValue 类型由 AX 约定，读取到栈上同布局结构。
    let ok = unsafe {
        AXValueGetValue(
            position.0,
            AX_VALUE_CG_POINT,
            &mut point as *mut Point as *mut c_void,
        ) && AXValueGetValue(
            size.0,
            AX_VALUE_CG_SIZE,
            &mut extent as *mut Size as *mut c_void,
        )
    };
    ok.then_some((point.x, point.y, extent.width, extent.height))
}

/// 两个矩形的四条边是否都在 3 点以内。
fn nearly_same(a: (i32, i32, u32, u32), b: (i32, i32, u32, u32)) -> bool {
    let edges = |r: (i32, i32, u32, u32)| [r.0, r.1, r.0 + r.2 as i32, r.1 + r.3 as i32];
    edges(a)
        .iter()
        .zip(edges(b).iter())
        .all(|(p, q)| (p - q).abs() <= 3)
}

pub(super) fn permission() -> HalResult<()> {
    // SAFETY: 无参数的只读查询，不会弹出授权窗口。
    if unsafe { AXIsProcessTrusted() } {
        Ok(())
    } else {
        Err(HalError::PermissionDenied {
            capability: "accessibility",
            guidance: "控件级智能选区需要辅助功能权限：请在「系统设置 → 隐私与安全性 → 辅助功能」中允许本应用，然后重新打开应用；未授权时仍可使用窗口级选区与手动框选".to_string(),
        })
    }
}

impl NativeOsHal {
    /// 光标处最前方外部窗口所属应用内的控件层级链（叶子在前），不含窗口本身。
    pub(super) fn ax_hierarchy_at_point(
        &self,
        x: i32,
        y: i32,
    ) -> HalResult<Vec<DetectedRegionRect>> {
        permission()?;
        let windows = self.native_windows()?;
        let Some(window) = windows.iter().find(|w| {
            x >= w.x && y >= w.y && x < w.x + w.width as i32 && y < w.y + w.height as i32
        }) else {
            return Ok(Vec::new());
        };
        // SAFETY: 由 PID 创建应用元素（+1 引用，CfOwned 负责释放）。
        let app = CfOwned(unsafe { AXUIElementCreateApplication(window.process_id as i32) });
        if app.0.is_null() {
            return Ok(Vec::new());
        }
        // SAFETY: app 为存活的 AXUIElement。
        unsafe { AXUIElementSetMessagingTimeout(app.0, MESSAGING_TIMEOUT_SECS) };
        let mut hit: *mut c_void = std::ptr::null_mut();
        // SAFETY: app 存活；hit 为输出参数，成功时为 +1 引用。
        let status =
            unsafe { AXUIElementCopyElementAtPosition(app.0, x as f32, y as f32, &mut hit) };
        if status != AX_SUCCESS || hit.is_null() {
            return Ok(Vec::new());
        }

        let mut chain = Vec::new();
        let mut current = CfOwned(hit);
        let mut last_rect: Option<(i32, i32, u32, u32)> = None;
        for depth in 0..MAX_DEPTH {
            let role = string_attribute(current.0, "AXRole").unwrap_or_default();
            // 到达窗口/应用层即停止：窗口边界由 L1 提供，避免重复且更准确（含阴影裁剪）。
            if role == "AXWindow" || role == "AXApplication" || role == "AXSheet" {
                break;
            }
            if let Some((fx, fy, fw, fh)) = frame_of(current.0) {
                let rect = (
                    fx.round() as i32,
                    fy.round() as i32,
                    fw.round().max(0.0) as u32,
                    fh.round().max(0.0) as u32,
                );
                // 跳过零尺寸与与子级几乎重合（各边相差 ≤3 点）的包装容器，让滚轮每一级都有可见变化。
                if rect.2 >= 4
                    && rect.3 >= 4
                    && !last_rect.is_some_and(|last| nearly_same(last, rect))
                {
                    let label = string_attribute(current.0, "AXRoleDescription")
                        .filter(|s| !s.trim().is_empty())
                        .or_else(|| (!role.is_empty()).then(|| role.clone()));
                    chain.push(DetectedRegionRect {
                        region_id: AX_REGION_BASE_ID + depth as u32,
                        parent_id: None,
                        level: if chain.is_empty() {
                            RegionHierarchyLevel::UiElement
                        } else {
                            RegionHierarchyLevel::ContainerPanel
                        },
                        x: rect.0,
                        y: rect.1,
                        width: rect.2,
                        height: rect.3,
                        label,
                    });
                    last_rect = Some(rect);
                }
            }
            match attribute(current.0, "AXParent") {
                Some(parent) => current = parent,
                None => break,
            }
        }
        Ok(chain)
    }
}
