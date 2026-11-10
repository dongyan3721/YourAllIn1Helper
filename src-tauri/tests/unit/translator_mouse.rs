use super::{clamped_popover_position, logical_position_from_physical, WindowsMonitorBounds};

#[cfg(target_os = "macos")]
use super::macos::is_selection_drag;

#[cfg(target_os = "macos")]
#[test]
fn translation_drag_ignores_clicks_focus_switches_and_own_windows() {
    let pid = std::process::id() as i32 + 100;
    assert!(!is_selection_drag((10.0, 20.0, pid), (10.0, 20.0, pid)));
    assert!(!is_selection_drag((10.0, 20.0, pid), (13.0, 23.0, pid)));
    assert!(is_selection_drag((10.0, 20.0, pid), (30.0, 20.0, pid)));
    assert!(!is_selection_drag((10.0, 20.0, pid), (30.0, 20.0, pid + 1)));
    assert!(!is_selection_drag((10.0, 20.0, 0), (30.0, 20.0, 0)));
    let own = std::process::id() as i32;
    assert!(!is_selection_drag((10.0, 20.0, own), (30.0, 20.0, own)));
}

#[test]
fn windows_popover_clamps_all_four_edges_to_work_area_with_real_dpi() {
    // 150% DPI 主屏：物理 2880x1620（逻辑 1920x1080）。
    // 四边都留出停靠区/任务栏边距：左 72px（逻辑 48）、上 54px（逻辑 36）、
    // 右 90px（逻辑 60）、下 108px（逻辑 72）。
    // 有效物理工作区：x=72..2790 (宽 2718), y=54..1512 (高 1458)
    // 有效逻辑工作区：left=48, top=36, right=1860, bottom=1008
    let monitor = WindowsMonitorBounds {
        x: 0,
        y: 0,
        width: 2880,
        height: 1620,
        work_x: 72,
        work_y: 54,
        work_width: 2718,
        work_height: 1458,
        scale_factor: 1.5,
    };
    let monitors = [monitor];
    let (w, h) = (420.0, 340.0);

    // 工作区内部坐标保持不变
    assert_eq!(
        clamped_popover_position(&monitors, 300.0, 200.0, w, h),
        (300.0, 200.0)
    );

    // 左边缘（落入左侧停靠栏）夹回逻辑 work_left = 48.0
    assert_eq!(
        clamped_popover_position(&monitors, 10.0, 200.0, w, h),
        (48.0, 200.0)
    );

    // 上边缘（落入顶部任务栏）夹回逻辑 work_top = 36.0
    assert_eq!(
        clamped_popover_position(&monitors, 300.0, 8.0, w, h),
        (300.0, 36.0)
    );

    // 右边缘（展开后越过右侧工作区边界）夹回 work_right - w = 1860 - 420 = 1440.0
    assert_eq!(
        clamped_popover_position(&monitors, 1700.0, 200.0, w, h),
        (1440.0, 200.0)
    );

    // 下边缘（展开后落入底部任务栏）夹回 work_bottom - h = 1008 - 340 = 668.0
    assert_eq!(
        clamped_popover_position(&monitors, 300.0, 990.0, w, h),
        (300.0, 668.0)
    );

    // 刚好落在显示器闭合右/下外边界点（1920.0, 1080.0）时同样命中并夹回工作区右下角
    assert_eq!(
        clamped_popover_position(&monitors, 1920.0, 1080.0, w, h),
        (1440.0, 668.0)
    );
}

#[test]
fn windows_popover_supports_negative_coordinate_secondary_monitors_and_mixed_dpi() {
    // 主屏：100% DPI，物理 (0, 0, 1920x1080)，工作区 (0, 0, 1920x1040)
    let primary = WindowsMonitorBounds {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
        work_x: 0,
        work_y: 0,
        work_width: 1920,
        work_height: 1040,
        scale_factor: 1.0,
    };
    // 左上负坐标副屏：200% DPI，物理 (-3840, -400, 3840x2160)
    // 物理工作区：左侧任务栏占 96px，底部任务栏占 96px -> (-3744, -400, 3744x2064)
    // 逻辑显示器范围：x in [-1920, 0), y in [-200, 880)
    // 逻辑工作区范围：left = -1872, top = -200, right = 0, bottom = 832
    let secondary = WindowsMonitorBounds {
        x: -3840,
        y: -400,
        width: 3840,
        height: 2160,
        work_x: -3744,
        work_y: -400,
        work_width: 3744,
        work_height: 2064,
        scale_factor: 2.0,
    };
    let monitors = [primary, secondary];
    let (w, h) = (420.0, 340.0);

    // 副屏左上角越界（进入左侧任务栏区域 x=-1910，贴顶 y=-200）
    assert_eq!(
        clamped_popover_position(&monitors, -1910.0, -200.0, w, h),
        (-1872.0, -200.0)
    );

    // 副屏右下角越界（靠近 x=0 的交界处与底部任务栏 y=850）
    // 应留在副屏内：x = 0 - 420 = -420, y = 832 - 340 = 492
    assert_eq!(
        clamped_popover_position(&monitors, -30.0, 850.0, w, h),
        (-420.0, 492.0)
    );

    // 主屏与副屏交界 x=0.0 属于主屏左边界，不应误吸到副屏
    assert_eq!(
        clamped_popover_position(&monitors, 0.0, 1020.0, w, h),
        (0.0, 700.0)
    );
}

#[test]
fn windows_popover_anchors_oversized_window_to_work_area_top_left() {
    // 负坐标高分副屏：逻辑工作区 [-1200, -100] x [-150, 550]（宽 1100，高 700）
    let monitor = WindowsMonitorBounds {
        x: -2400,
        y: -300,
        width: 2200,
        height: 1500,
        work_x: -2400,
        work_y: -300,
        work_width: 2200,
        work_height: 1400,
        scale_factor: 2.0,
    };
    let monitors = [monitor];

    // 气泡宽高均大于工作区宽高时，不得 panic，且固定到工作区左上角 (-1200.0, -150.0)
    assert_eq!(
        clamped_popover_position(&monitors, -500.0, 100.0, 1600.0, 900.0),
        (-1200.0, -150.0)
    );
    // 仅宽度过大、高度正常
    assert_eq!(
        clamped_popover_position(&monitors, -500.0, 500.0, 1600.0, 300.0),
        (-1200.0, 250.0)
    );
    // 仅高度过大、宽度正常
    assert_eq!(
        clamped_popover_position(&monitors, -200.0, 100.0, 400.0, 900.0),
        (-500.0, -150.0)
    );
}

#[test]
fn windows_popover_handles_abnormal_inputs_without_panicking() {
    let valid_monitor = WindowsMonitorBounds {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
        work_x: 0,
        work_y: 40,
        work_width: 1920,
        work_height: 1000,
        scale_factor: 1.0,
    };

    // 1. 空显示器列表：原样返回有限坐标；非有限坐标退回 0.0
    assert_eq!(
        clamped_popover_position(&[], 320.0, 240.0, 420.0, 340.0),
        (320.0, 240.0)
    );
    assert_eq!(
        clamped_popover_position(&[], f64::NAN, f64::INFINITY, 420.0, 340.0),
        (0.0, 0.0)
    );

    // 2. 非法缩放因子（0、负数、NaN、无穷大）与零尺寸显示器应被忽略，不除零不 panic
    let invalid_monitors = [
        WindowsMonitorBounds {
            scale_factor: 0.0,
            ..valid_monitor
        },
        WindowsMonitorBounds {
            scale_factor: -1.5,
            ..valid_monitor
        },
        WindowsMonitorBounds {
            scale_factor: f64::NAN,
            ..valid_monitor
        },
        WindowsMonitorBounds {
            scale_factor: f64::INFINITY,
            ..valid_monitor
        },
        WindowsMonitorBounds {
            width: 0,
            ..valid_monitor
        },
        WindowsMonitorBounds {
            height: 0,
            ..valid_monitor
        },
    ];
    assert_eq!(
        clamped_popover_position(&invalid_monitors, 320.0, 240.0, 420.0, 340.0),
        (320.0, 240.0)
    );
    assert_eq!(
        logical_position_from_physical(&invalid_monitors, 500, 400, 20.0, 20.0),
        (500.0, 400.0)
    );

    // 3. 异常窗口宽高（0、负数、NaN、±Infinity）
    let monitors = [valid_monitor];
    assert_eq!(
        clamped_popover_position(&monitors, 1910.0, 1030.0, -200.0, 0.0),
        (1910.0, 1030.0)
    );
    assert_eq!(
        clamped_popover_position(&monitors, 1910.0, 1030.0, f64::NAN, f64::NEG_INFINITY),
        (1910.0, 1030.0)
    );
    assert_eq!(
        clamped_popover_position(&monitors, 500.0, 500.0, f64::INFINITY, f64::INFINITY),
        (0.0, 40.0)
    );

    // 4. 非有限坐标（NaN / ±Infinity）落回首个有效显示器的工作区安全位置
    assert_eq!(
        clamped_popover_position(&monitors, f64::NAN, f64::INFINITY, 420.0, 340.0),
        (0.0, 40.0)
    );

    // 5. 异常 work_area（尺寸为 0 或完全越出显示器）退回显示器完整范围
    let zero_work_area = WindowsMonitorBounds {
        work_x: 5000,
        work_y: 5000,
        work_width: 0,
        work_height: 0,
        ..valid_monitor
    };
    assert_eq!(
        clamped_popover_position(&[zero_work_area], 1800.0, 950.0, 420.0, 340.0),
        (1500.0, 740.0)
    );

    // 6. 不在任何显示器内的有限坐标保持原值返回
    assert_eq!(
        clamped_popover_position(&monitors, 5000.0, -3000.0, 420.0, 340.0),
        (5000.0, -3000.0)
    );
}

#[test]
fn windows_physical_mouse_to_logical_trigger_to_tauri_popover_does_not_double_scale_dpi() {
    // 复现完整链路：
    // 1) Windows 鼠标钩子上报物理像素 (phys_x, phys_y)
    // 2) logical_position_from_physical 转换为 20x20 触发按钮的逻辑坐标 (trigger_x, trigger_y)
    // 3) 展开气泡时 tauri_app.rs 调用 clamped_popover_position 得到气泡逻辑坐标 (popover_x, popover_y)
    // 4) Tauri WebviewWindowBuilder::position 接收逻辑坐标并在建窗时乘以显示器 scale_factor 还原物理像素
    let primary_125 = WindowsMonitorBounds {
        x: 0,
        y: 0,
        width: 2400,
        height: 1350,
        work_x: 0,
        work_y: 0,
        work_width: 2400,
        work_height: 1290, // 底部任务栏 60px 物理像素（48px 逻辑像素）
        scale_factor: 1.25,
    };
    let secondary_neg_200 = WindowsMonitorBounds {
        x: -3840,
        y: -200,
        width: 3840,
        height: 2160,
        work_x: -3840,
        work_y: -200,
        work_width: 3840,
        work_height: 2064, // 底部任务栏 96px 物理像素（48px 逻辑像素）
        scale_factor: 2.0,
    };
    let monitors = [primary_125, secondary_neg_200];
    let (popover_w, popover_h) = (420.0, 340.0);

    // 场景 A：125% 主屏中间位置划词（物理 625, 500）
    let (trig_x, trig_y) = logical_position_from_physical(&monitors, 625, 500, 20.0, 20.0);
    assert_eq!(
        (trig_x, trig_y),
        ((625.0 + 8.0) / 1.25, (500.0 + 8.0) / 1.25)
    );
    let (pop_x, pop_y) = clamped_popover_position(&monitors, trig_x, trig_y, popover_w, popover_h);
    // 未越界时不应再次除以或乘以 1.25
    assert_eq!((pop_x, pop_y), (trig_x, trig_y));
    let tauri_phys_x = (pop_x * primary_125.scale_factor).round() as i32;
    let tauri_phys_y = (pop_y * primary_125.scale_factor).round() as i32;
    assert_eq!((tauri_phys_x, tauri_phys_y), (633, 508));

    // 场景 B：200% 负坐标副屏右下角靠近任务栏划词（物理 -20, 1855）
    let (trig_x, trig_y) = logical_position_from_physical(&monitors, -20, 1855, 20.0, 20.0);
    // 20x20 触发按钮逻辑坐标已被夹在副屏逻辑工作区 [-1920..0, -100..932] 内：(-20, 912)
    assert_eq!((trig_x, trig_y), (-20.0, 912.0));
    let (pop_x, pop_y) = clamped_popover_position(&monitors, trig_x, trig_y, popover_w, popover_h);
    // 展开为 420x340 后逻辑坐标应夹到 (-420, 932 - 340 = 592)
    assert_eq!((pop_x, pop_y), (-420.0, 592.0));

    // Tauri 建窗乘回 2.0 DPI 后的物理矩形必须严格落在副屏物理工作区 [-3840..0, -200..1864] 内
    let win_phys_left = (pop_x * secondary_neg_200.scale_factor).round() as i32;
    let win_phys_top = (pop_y * secondary_neg_200.scale_factor).round() as i32;
    let win_phys_right =
        win_phys_left + (popover_w * secondary_neg_200.scale_factor).round() as i32;
    let win_phys_bottom =
        win_phys_top + (popover_h * secondary_neg_200.scale_factor).round() as i32;
    assert_eq!((win_phys_left, win_phys_top), (-840, 1184));
    assert_eq!(
        win_phys_right,
        secondary_neg_200.work_x + secondary_neg_200.work_width as i32
    );
    assert_eq!(
        win_phys_bottom,
        secondary_neg_200.work_y + secondary_neg_200.work_height as i32
    );
}
