# Windows 翻译气泡工作区边界约束（2026-11-10）

- **分支 / 工作目录**：`codex/windows-translator-bounds` / `D:\YourAllIn1Helper-worktrees\translator-bounds`
- **基线提交**：`f7fb8fc`（`docs: record Windows GUI fixes and final validation`）
- **改动范围**：
  - `src-tauri/src/translator_mouse.rs`
  - `src-tauri/tests/unit/translator_mouse.rs`
  - `docs/testing/WINDOWS_TRANSLATOR_BOUNDS_2026-11-10.md`
- **范围声明**：未修改 C 盘主目录或切换主目录分支；未修改 `src-tauri/src/tauri_app.rs`、`Cargo.toml`、`Cargo.lock` 或前端文件；未启动桌面 GUI、未触发全局热键。

---

## 1. 问题与根因

1. `src-tauri/src/tauri_app.rs` 在创建 `translate-popover` 窗口时调用 `crate::translator_mouse::popover_position(&self.app, x, y, spec.width, spec.height)` 并把返回值传给 `WebviewWindowBuilder::position(x, y)`。
2. 原实现中 `popover_position` 仅在 `#[cfg(target_os = "macos")]` 调用 `macos::clamped_position`，在 Windows 下直接原样返回 `(x, y)`：
   - 鼠标划词入口只按 `20×20` 触发按钮尺寸及整屏尺寸做过一次初始夹取；当用户点击触发按钮展开为完整翻译气泡（默认 `420×340`）时，靠近屏幕右边缘、下边缘或任务栏处会越出显示器工作区或被任务栏遮挡。
   - 全局快捷键入口（默认 `(300.0, 200.0)`）或超大气泡尺寸同样未按 Windows 显示器工作区约束。
3. **DPI 与坐标空间契约**：
   - Windows 鼠标钩子采集的是物理像素坐标 `(x: i32, y: i32)`，`logical_position` 已将其按目标显示器的 `scale_factor` 换算为逻辑坐标 `(x: f64, y: f64)` 存入选区会话。
   - Tauri 在 Windows 上报告的 `Monitor::position()`、`Monitor::size()` 与 `Monitor::work_area()` 均为物理像素（`PhysicalPosition<i32>` / `PhysicalSize<u32>` / `PhysicalRect<i32, u32>`），而 `WebviewWindowBuilder::position(x, y)` 接收 `LogicalPosition` 并在底层 `tao` 建窗时乘以目标显示器的 `scale_factor` 还原为物理坐标。
   - 若在 `popover_position` 中把输入的逻辑坐标再次除以 `scale_factor`，或直接返回物理工作区坐标，都会导致 Tauri 建窗阶段出现二次 DPI 换算偏差。

---

## 2. 实现说明

### 2.1 `src-tauri/src/translator_mouse.rs`
- 新增纯数据结构 `WindowsMonitorBounds`（含显示器物理起点/宽高、`work_area` 物理起点/宽高及 `scale_factor`），通过 `WindowsMonitorBounds::from_monitor(&tauri::Monitor)` 从 Tauri 显示器描述提取。
- `logical_monitor_bounds()` 与 `logical_work_bounds()`：
  - 校验显示器有效性（`width > 0 && height > 0 && scale_factor.is_finite() && scale_factor > 0.0`）；
  - 将物理 `work_area` 与显示器物理矩形求交（若 `work_area` 尺寸为 0 或异常越出显示器，则退回显示器完整矩形），再统一除以 `scale_factor` 得到逻辑工作区 `(work_left, work_top, work_right, work_bottom)`。
- `logical_position_from_physical()`：
  - 按物理鼠标坐标 `(x, y)` 命中所属显示器，将 `(x + 8, y + 8)` 除以该显示器的 `scale_factor` 转为逻辑坐标，并夹取在逻辑工作区内。
- `clamped_popover_position()` 与 `windows::clamped_position()`：
  - 接收已经是逻辑坐标的 `(x, y)` 与逻辑窗口宽高 `(width, height)`；
  - 优先按半开区间 `[left, right) × [top, bottom)` 匹配所属显示器逻辑范围，未命中时再用闭合右/下边界 `[left, right] × [top, bottom]` 兜底外边缘点；
  - 在目标显示器的逻辑工作区内执行 `x.clamp(work_left, (work_right - clean_width).max(work_left))` 与 `y.clamp(work_top, (work_bottom - clean_height).max(work_top))`，返回逻辑坐标供 `builder.position(x, y)` 使用。
- `popover_position()`：
  - `#[cfg(target_os = "macos")]` 保持原有 `macos::clamped_position` 行为不变；
  - `#[cfg(target_os = "windows")]` 调用 `windows::clamped_position(app, x, y, width, height)`。
- 单元测试挂载条件从 `#[cfg(all(test, target_os = "macos"))]` 放宽为 `#[cfg(test)]`，保留原 macOS 拖选测试的 `#[cfg(target_os = "macos")]` 条件。

### 2.2 `src-tauri/tests/unit/translator_mouse.rs`
新增 5 组 Windows 纯逻辑回归测试：
1. `windows_popover_clamps_all_four_edges_to_work_area_with_real_dpi`：覆盖 150% DPI 下左、上、右、下四边工作区/任务栏停靠夹取及右/下闭合外边界点。
2. `windows_popover_supports_negative_coordinate_secondary_monitors_and_mixed_dpi`：覆盖负坐标 200% DPI 副屏与 100% 主屏组合下的左上、右下工作区约束及 `x = 0` 屏幕交界点归属。
3. `windows_popover_anchors_oversized_window_to_work_area_top_left`：覆盖宽、高或双向超过逻辑工作区尺寸的过大窗口，验证不触发 `clamp` panic 且固定到逻辑工作区左上角 `(work_left, work_top)`。
4. `windows_popover_handles_abnormal_inputs_without_panicking`：覆盖空显示器列表、非法 `scale_factor`（`0.0`、负数、`NaN`、`Infinity`）、零尺寸显示器、零尺寸/越界 `work_area` 回退、异常窗口尺寸（`0.0`、负数、`NaN`、`±Infinity`）、非有限坐标（`NaN`、`±Infinity`）及不属任何显示器的有限坐标。
5. `windows_physical_mouse_to_logical_trigger_to_tauri_popover_does_not_double_scale_dpi`：串联物理鼠标坐标 → `logical_position_from_physical`（`20×20` 触发窗）→ `clamped_popover_position`（`420×340` 展开气泡）→ Tauri 建窗乘回 `scale_factor` 的完整链路，确认无二次 DPI 乘除且最终物理窗口矩形严格落在物理 `work_area` 内。

---

## 3. 已知局限与未验证项

1. **混合 DPI 下全局逻辑坐标重叠的既有歧义未在本层消除**：
   - 由于 `WindowSpec` 与 `WebviewWindowBuilder::position(x, y)` 仅传递不带显示器标识的二维逻辑坐标 `(f64, f64)`，而各显示器独立按自身 `scale_factor` 将物理矩形映射为 `[pos / scale, (pos + size) / scale)`，在部分混合 DPI 排列下（例如左侧主屏 `0..1920` 为 100% 缩放、逻辑横坐标 `[0, 1920)`，右侧副屏物理起点 `1920` 为 200% 缩放、逻辑横坐标 `[960, 2880)`），不同显示器的逻辑坐标区间在 `[960, 1920)` 重叠。
   - 当前 `clamped_popover_position`（以及底层 `tao` 的 `WindowBuilder::position`）按 `available_monitors()` 顺序匹配首个包含 `(x, y)` 的显示器，在重叠区间内可能命中前序显示器而非原触发显示器。要彻底消除该歧义需跨模块传递 `monitor_id` 或改用物理坐标建窗接口，不在本次最小改动范围内。
2. **测试执行状态（未在冷缓存 worktree 跑完，不得记为通过）**：
   - 已对改动文件执行 `rustfmt --edition 2021 --check src-tauri/src/translator_mouse.rs src-tauri/tests/unit/translator_mouse.rs`，检查通过（Exit Code 0）。
   - 在 D 盘独立 worktree 启动了 `cargo test -j 1 -p all-in-one-host --lib translator_mouse`，因冷缓存从头编译依赖耗时，已按协调指令主动停止后台编译任务（`task-78`），**未完成编译与单测运行，不能记为测试通过**；交由 Codex 在主目录复用已有编译缓存统一执行相关单测与最终桌面构建验证。
3. **真实桌面与多屏 GUI 未验收**：
   - 本轮未启动桌面应用，未在真实 Windows 单屏/多屏/混合 DPI/不同任务栏停靠位置下做 GUI 实测。
