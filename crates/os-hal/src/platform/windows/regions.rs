//! Windows UIAutomation 查询；所有 COM 与 DPI 状态都限制在专用工作线程。

use super::super::native_error;
use crate::{HalError, HalResult};
use app_core::{DetectedRegionRect, RegionHierarchyLevel};
use std::{collections::HashSet, thread};
use windows::{
    core::{Interface, BOOL, HRESULT},
    Win32::{
        Foundation::{HWND, LPARAM, RECT},
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_MULTITHREADED,
        },
        UI::{
            Accessibility::{
                CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTreeWalker,
                UIA_E_ELEMENTNOTAVAILABLE,
            },
            HiDpi::{
                SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            },
            WindowsAndMessaging::{
                EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
            },
        },
    },
};

const MAX_TREE_DEPTH: usize = 32;
const MAX_VISITED_ELEMENTS: usize = 1024;
const REGION_ID_BASE: u32 = 50_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct PhysicalRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl PhysicalRect {
    fn from_win32(rect: RECT) -> Option<Self> {
        (rect.right > rect.left && rect.bottom > rect.top).then_some(Self {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        })
    }

    fn contains(self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }

    fn intersection(self, other: Self) -> Option<Self> {
        let clipped = Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        (clipped.right > clipped.left && clipped.bottom > clipped.top).then_some(clipped)
    }

    fn region_size(self) -> Option<(u32, u32)> {
        let width = self.right as i64 - self.left as i64;
        let height = self.bottom as i64 - self.top as i64;
        (width > 0 && height > 0 && width <= u32::MAX as i64 && height <= u32::MAX as i64)
            .then_some((width as u32, height as u32))
    }
}

#[derive(Clone, Debug)]
struct UiNode {
    identity: u64,
    rect: PhysicalRect,
    label: Option<String>,
}

#[derive(Clone, Copy)]
struct TargetWindow {
    hwnd: HWND,
    process_id: u32,
    rect: PhysicalRect,
}

struct ComApartment;

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct DpiContext(DPI_AWARENESS_CONTEXT);

impl DpiContext {
    fn enter() -> HalResult<Self> {
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if previous.0.is_null() {
            return Err(native_error(
                "无法将 UIAutomation 工作线程切换为每显示器 DPI 感知模式",
            ));
        }
        Ok(Self(previous))
    }
}

impl Drop for DpiContext {
    fn drop(&mut self) {
        unsafe { SetThreadDpiAwarenessContext(self.0) };
    }
}

/// 查询前台 Z 序中光标所在的首个外部窗口，并返回叶子控件至窗口的真实 UIA 链。
pub(in crate::platform) fn query_hierarchy_at_point(
    x: i32,
    y: i32,
) -> HalResult<Vec<DetectedRegionRect>> {
    thread::Builder::new()
        .name("windows-region-uia".into())
        .spawn(move || query_on_worker(x, y))
        .map_err(|error| {
            HalError::NativeCallFailed(format!("启动 UIAutomation 工作线程失败：{error}"))
        })?
        .join()
        .map_err(|_| HalError::NativeCallFailed("UIAutomation 工作线程意外终止".into()))?
}

fn query_on_worker(x: i32, y: i32) -> HalResult<Vec<DetectedRegionRect>> {
    let _dpi = DpiContext::enter()?;
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|error| {
                HalError::NativeCallFailed(format!("初始化 UIAutomation COM 失败：{error}"))
            })?;
    }
    let _com = ComApartment;

    let target = topmost_external_window_at(x, y)?
        .ok_or_else(|| HalError::NotFound(format!("坐标 ({x}, {y}) 没有外部窗口")))?;
    let automation: IUIAutomation = unsafe {
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).map_err(|error| {
            HalError::NativeCallFailed(format!("创建 UIAutomation 客户端失败：{error}"))
        })?
    };
    let root = unsafe { automation.ElementFromHandle(target.hwnd) }
        .map_err(|error| map_uia_error("无法读取目标窗口的 UIAutomation 根节点", error))?;
    let root_process_id = unsafe { root.CurrentProcessId() }
        .map_err(|error| map_uia_error("读取 UIAutomation 窗口进程失败", error))?;
    if root_process_id <= 0 || root_process_id as u32 != target.process_id {
        return Err(HalError::NotFound(
            "UIAutomation 窗口进程与命中的原生 HWND 不匹配".into(),
        ));
    }
    let walker = unsafe { automation.ControlViewWalker() }
        .map_err(|error| map_uia_error("无法创建 UIAutomation 控件树遍历器", error))?;
    let mut budget = 0;
    let leaf = find_hit_element(&root, &walker, x, y, 0, &mut budget)?
        .ok_or(HalError::Unsupported("windows_uiautomation_hit_element"))?;

    let root_node = UiNode {
        identity: 0,
        rect: target.rect,
        label: element_name(&root),
    };
    let mut chain = Vec::new();
    // CompareElements 使用 UIA provider 身份；保留所有已比较的代理引用，避免地址复用误判。
    let mut held_elements = vec![root.clone()];
    let mut current = leaf;
    for _ in 0..MAX_TREE_DEPTH {
        if contains_element(&automation, &held_elements, &current)? {
            break;
        }
        held_elements.push(current.clone());
        if let Some(rect) = element_rect(&current)? {
            chain.push(UiNode {
                identity: chain.len() as u64 + 1,
                rect,
                label: element_name(&current),
            });
        }
        let Some(parent) = walker_element(&walker, &current, WalkerRelation::Parent)? else {
            break;
        };
        current = parent;
    }
    if chain.len() == MAX_TREE_DEPTH {
        return Err(HalError::NativeCallFailed(
            "UIAutomation 父级链超过安全深度上限".into(),
        ));
    }
    regions_from_chain(&chain, &root_node)
}

fn topmost_external_window_at(x: i32, y: i32) -> HalResult<Option<TargetWindow>> {
    struct Enumeration {
        x: i32,
        y: i32,
        own_pid: u32,
        target: Option<TargetWindow>,
        error: Option<String>,
    }

    unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> BOOL {
        let state = &mut *(data.0 as *mut Enumeration);
        if state.target.is_some() {
            return BOOL(1);
        }
        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }
        let mut process_id = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
        if process_id == 0 || process_id == state.own_pid {
            return BOOL(1);
        }
        let mut rect = RECT::default();
        if let Err(error) = GetWindowRect(hwnd, &mut rect) {
            state.error = Some(format!("读取外部窗口边界失败：{error}"));
            return BOOL(0);
        }
        let Some(rect) = PhysicalRect::from_win32(rect) else {
            return BOOL(1);
        };
        if rect.contains(state.x, state.y) {
            state.target = Some(TargetWindow {
                hwnd,
                process_id,
                rect,
            });
            return BOOL(0);
        }
        BOOL(1)
    }

    let mut state = Enumeration {
        x,
        y,
        own_pid: std::process::id(),
        target: None,
        error: None,
    };
    let enumerated =
        unsafe { EnumWindows(Some(visit), LPARAM(&mut state as *mut Enumeration as isize)) };
    if let Some(error) = state.error {
        return Err(HalError::NativeCallFailed(error));
    }
    if state.target.is_some() {
        return Ok(state.target);
    }
    enumerated.map_err(native_error)?;
    Ok(state.target)
}

fn find_hit_element(
    element: &IUIAutomationElement,
    walker: &IUIAutomationTreeWalker,
    x: i32,
    y: i32,
    depth: usize,
    budget: &mut usize,
) -> HalResult<Option<IUIAutomationElement>> {
    if depth >= MAX_TREE_DEPTH {
        return Err(HalError::NativeCallFailed(
            "UIAutomation 控件树超过安全深度上限".into(),
        ));
    }
    let Some(rect) = element_rect(element)? else {
        return Ok(None);
    };
    if !rect.contains(x, y) {
        return Ok(None);
    }
    // 即使 provider 返回循环的同级链，也会被总访问节点上限截断。
    *budget += 1;
    if *budget > MAX_VISITED_ELEMENTS {
        return Err(HalError::NativeCallFailed(
            "UIAutomation 控件树超过单次查询节点上限".into(),
        ));
    }

    let mut best = Some(element.clone());
    let Some(mut child) = walker_element(walker, element, WalkerRelation::FirstChild)? else {
        return Ok(best);
    };
    loop {
        *budget += 1;
        if *budget > MAX_VISITED_ELEMENTS {
            return Err(HalError::NativeCallFailed(
                "UIAutomation 控件树超过单次查询节点上限".into(),
            ));
        }
        if element_rect(&child)?.is_some_and(|rect| rect.contains(x, y)) {
            if let Some(hit) = find_hit_element(&child, walker, x, y, depth + 1, budget)? {
                best = Some(hit);
            }
        }
        let Some(sibling) = walker_element(walker, &child, WalkerRelation::NextSibling)? else {
            break;
        };
        child = sibling;
    }
    Ok(best)
}

#[derive(Clone, Copy)]
enum WalkerRelation {
    Parent,
    FirstChild,
    NextSibling,
}

fn walker_element(
    walker: &IUIAutomationTreeWalker,
    element: &IUIAutomationElement,
    relation: WalkerRelation,
) -> HalResult<Option<IUIAutomationElement>> {
    let method = match relation {
        WalkerRelation::Parent => Interface::vtable(walker).GetParentElement,
        WalkerRelation::FirstChild => Interface::vtable(walker).GetFirstChildElement,
        WalkerRelation::NextSibling => Interface::vtable(walker).GetNextSiblingElement,
    };
    let mut result = std::ptr::null_mut();
    let status = unsafe { method(walker.as_raw(), element.as_raw(), &mut result) };
    let has_value = match interface_result_has_value(
        status,
        result.is_null(),
        "调用 UIAutomation 控件树遍历器失败",
    ) {
        Ok(has_value) => has_value,
        Err(error) => {
            if !result.is_null() {
                drop(unsafe { IUIAutomationElement::from_raw(result) });
            }
            return Err(error);
        }
    };
    if !has_value {
        return Ok(None);
    }
    Ok(Some(unsafe { IUIAutomationElement::from_raw(result) }))
}

fn interface_result_has_value(status: HRESULT, is_null: bool, context: &str) -> HalResult<bool> {
    if status.0 < 0 {
        return Err(map_uia_hresult(context, status));
    }
    Ok(!is_null)
}

fn contains_element(
    automation: &IUIAutomation,
    held_elements: &[IUIAutomationElement],
    candidate: &IUIAutomationElement,
) -> HalResult<bool> {
    for existing in held_elements {
        let equal = unsafe { automation.CompareElements(existing, candidate) }
            .map_err(|error| map_uia_error("比较 UIAutomation 控件身份失败", error))?;
        if equal.as_bool() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn element_rect(element: &IUIAutomationElement) -> HalResult<Option<PhysicalRect>> {
    let rect = unsafe { element.CurrentBoundingRectangle() }
        .map_err(|error| map_uia_error("读取 UIAutomation 控件物理边界失败", error))?;
    Ok(PhysicalRect::from_win32(rect))
}

fn element_name(element: &IUIAutomationElement) -> Option<String> {
    unsafe { element.CurrentName() }
        .ok()
        .map(|name| name.to_string())
        .filter(|name| !name.trim().is_empty())
}

fn map_uia_error(context: &str, error: windows::core::Error) -> HalError {
    map_uia_hresult(context, error.code())
}

fn map_uia_hresult(context: &str, status: HRESULT) -> HalError {
    if status.0 as u32 == UIA_E_ELEMENTNOTAVAILABLE {
        HalError::Unsupported("windows_uiautomation_provider")
    } else {
        HalError::NativeCallFailed(format!("{context}：HRESULT 0x{:08X}", status.0 as u32))
    }
}

fn regions_from_chain(nodes: &[UiNode], root: &UiNode) -> HalResult<Vec<DetectedRegionRect>> {
    let mut seen_identities = HashSet::new();
    let mut seen_rects = HashSet::new();
    seen_rects.insert(root.rect);
    let mut visible_nodes = Vec::new();

    for node in nodes {
        if node.identity == root.identity || !seen_identities.insert(node.identity) {
            break;
        }
        let Some(rect) = node.rect.intersection(root.rect) else {
            continue;
        };
        let Some((width, height)) = rect.region_size() else {
            continue;
        };
        if !seen_rects.insert(rect) {
            continue;
        }
        visible_nodes.push((node, rect, width, height));
    }
    if visible_nodes.is_empty() {
        return Err(HalError::Unsupported("windows_uiautomation_hit_element"));
    }
    let Some((root_width, root_height)) = root.rect.region_size() else {
        return Err(HalError::Unsupported("windows_uiautomation_window_bounds"));
    };

    let mut regions = Vec::with_capacity(visible_nodes.len() + 1);
    for (index, (node, rect, width, height)) in visible_nodes.into_iter().enumerate() {
        let region_id = REGION_ID_BASE + index as u32;
        regions.push(DetectedRegionRect {
            region_id,
            parent_id: Some(region_id + 1),
            level: if index == 0 {
                RegionHierarchyLevel::UiElement
            } else {
                RegionHierarchyLevel::ContainerPanel
            },
            x: rect.left,
            y: rect.top,
            width,
            height,
            label: node.label.clone(),
        });
    }
    let root_id = REGION_ID_BASE + regions.len() as u32;
    regions.push(DetectedRegionRect {
        region_id: root_id,
        parent_id: None,
        level: RegionHierarchyLevel::TopLevelWindow,
        x: root.rect.left,
        y: root.rect.top,
        width: root_width,
        height: root_height,
        label: root.label.clone(),
    });
    Ok(regions)
}

#[cfg(test)]
#[path = "../../../tests/unit/windows_regions.rs"]
mod tests;
