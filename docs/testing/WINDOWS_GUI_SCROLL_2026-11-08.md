# Windows GUI 修复测试报告：长截图蓝框污染与滚轮处理（2026-11-08）

## 1. 任务基本信息

- **智能体 / 开发者**：Antigravity / Gemini 3.8 Flash
- **认领任务**：长截图蓝框污染修复、滚轮透传与边界安全加固、GIF UI 配合
- **工作目录**：`D:\YourAllIn1Helper-worktrees\scroll`
- **分支名称**：`codex/windows-scroll-overlay`
- **基线 Commit**：`0cdada8`（分支基础提交：`a2c130c docs: coordinate isolated Windows GUI repair branches`）

---

## 2. 缺陷根因分析（Root Cause Analysis）

长截图在向下滚动拼接时，拼接产物中间出现周期性水平蓝色横线、两侧出现垂直蓝线（即“蓝框入图污染”），以及滚轮滚动未穿透的根本原因如下：

1. **选区边框向内侵入选区内部**：
   在 `src-ui/src/components/CaptureOverlay.vue` 的 `renderUi()` 中，原本采用如下方式在 `uiCanvas` 上绘制选区外框：
   ```typescript
   ctx.strokeRect(
     focus.x + ctx.lineWidth / 2,
     focus.y + ctx.lineWidth / 2,
     focus.width - ctx.lineWidth,
     focus.height - ctx.lineWidth
   );
   ```
   因为 Canvas 2D 描边沿路径中心居中向内外两侧各扩展 `lineWidth / 2`，导致蓝色边框（1.5px ~ 2px）完全或部分绘制在选区矩形内部。
2. **手柄与标签 DOM 元素侵入捕获区域**：
   选区四角和边中点共 8 个调整手柄（`.handle`）、尺寸标签（`.size-badge`）以及提示信息（`.region-label` / `.capture-hint`）在进入 `liveMode === 'scroll'` 时未被隐藏，其 DOM 节点覆盖在选区边缘上。
3. **滚动拼接算法逐帧提取底部像素**：
   长截图拼接器 `ScrollStitcher::push_frame` 采用 NCC 计算相邻帧位移量 $\Delta y$，向下滚动时从当前帧提取底部的 $\Delta y$ 行（`height - fixed_footer - dy .. height - fixed_footer`）追加到大图。由于选区底边存在绘制在内侧的蓝色底边线，每一次滚动追加都会把底部的蓝色边框行拼接进最终图像，形成周期性水平蓝线；而左右两侧边缘的蓝色描边则在全高范围内拼接成连续垂直蓝线。
4. **滚轮事件被前端默认拦截**：
   模板中绑定了 `@wheel.prevent="onWheel"`，在长截图采集中未能将滚轮事件透传给下层真实滚动窗口；同时 CSS 未显式声明透明及透传样式。

---

## 3. 修复实现方案与技术细节

根据协同规范：**“蓝框污染应从绘制/捕获边界修复，不能裁去用户真实选择区的像素来掩盖”**。本轮修复坚持 100% 保留真实选区几何尺寸与像素，不裁切任何用户选区像素：

1. **严格外围描边与次像素抗锯齿擦除**（`src-ui/src/utils/screenshot/scroll.ts` & `CaptureOverlay.vue`）：
   - 新增 `outerBorderRect(rect, lineWidth)`：根据描边线宽计算外扩坐标 `x - lineWidth / 2`，`width + lineWidth`，使描边中心完全落在选区外围的暗色遮罩层上，内边缘与选区边界无缝相切。
   - 在 `renderUi()` 中调用 `ctx.strokeRect(...)` 后紧接着执行 `ctx.clearRect(focus.x, focus.y, focus.width, focus.height)`，彻底擦除任何可能由抗锯齿算法向内溢出的次像素蓝晕，确保选区内部 100% 纯净全透。
2. **实时采集模式彻底隐藏手柄与辅助 DOM**（`CaptureOverlay.vue`）：
   - 手柄模板增加 `!liveMode` 条件（`v-if="selectionCss && !exporting && !liveMode"`），尺寸标签、智能区域提示均增加 `!liveMode` 判定，保证长截图与 GIF 录制时选区无任何 DOM 阻挡。
3. **滚轮事件透传与透明通道**（`CaptureOverlay.vue`）：
   - 模板去除 `.prevent` 修饰符，改为 `@wheel="onWheel"`。
   - 新增 `shouldPassthroughWheel(liveMode)`：当 `liveMode === 'scroll'` 时立即 `return`（不执行 `e.preventDefault()`），允许滚轮自然透传至下层桌面窗口。
   - 在 `startScrollCapture` 挖洞 `setLiveHole(true)` 后立即触发 `requestRender()`；在 `stopScrollCapture` 和 `cancelScrollCapture` 中规范调用 `setLiveHole(false)`、`requestRender()`、清理 blob URL，并在 `finally` 中复位 `scrollStopping = false`。
   - CSS 中为 `.capture-root.live-hole` 及 `body.capture-overlay-body.live-hole` 声明 `background: transparent !important;`。
4. **GIF UI 协同**（`CaptureOverlay.vue`）：
   - 为 `.live-bar` 显式设置 `pointer-events: auto !important;` 与 `z-index: 22`，确保 GIF 录制时的停止/取消浮动工具栏始终可响应点击交互。
5. **Rust 拼接器边界算术溢出防护**（`crates/mod-screenshot/src/scroll/stitcher.rs`）：
   - 将 `height - self.fixed_footer - dy`、`height - self.fixed_footer` 与 `footer_start` 重构为 `saturating_sub`，杜绝极端微小尺寸或异常页脚参数下的整数溢出 panic。

---

## 4. 自动化测试与验证

### 4.1 前端单元测试（Vitest / Node.js native test runner）

执行命令：
```powershell
Set-Location "D:\YourAllIn1Helper-worktrees\scroll"
pnpm --dir src-ui test
```

测试结果：
- **74 passed, 0 failed**
- 覆盖项：
  - `outerBorderRect` 在整数与高分屏（DPR = 1.5 等）浮点线宽下的精确外扩计算。
  - `shouldPassthroughWheel` 在 `'scroll'`、`'gif'` 及 `null` 状态下的分支逻辑。
  - 既有 72 项截图、标注、遮罩初始化、便签、密码库测试全部无回归通过。

### 4.2 Rust 单元与集成测试（mod-screenshot）

执行命令：
```powershell
Set-Location "D:\YourAllIn1Helper-worktrees\scroll"
cargo test -p mod-screenshot -j 1 --target-dir "D:\YourAllIn1Helper-worktrees\scroll\target"
```

测试结果：
- **62 passed, 0 failed, 0 ignored**
  - `src/lib.rs` (unit tests): **41 passed**
  - `tests/lifecycle.rs`: **18 passed**
  - `tests/m3_lifecycle.rs`: **3 passed**
- 新增专项测试：
  - `test_scroll_edge_pixels_preserved_without_cropping_or_pollution`：验证在多步向下滚动模拟中，首列（$x = 0$）、尾列（$x = \text{width} - 1$）及每一行像素均 100% 逐像素严格匹配原图，绝不裁切用户任何真实边缘像素，绝无边框污染。
  - `test_scroll_boundary_saturating_arithmetic_prevents_overflow`：验证小尺寸与极端页脚/位移参数下无算术下溢 panic。

---

## 5. 未测项与验收说明（Unmeasured Items）

1. **原生桌面 GUI 操作**：按照 `docs/WINDOWS_GUI_FIXES_COORDINATION_2026-11-08.md` 协调要求第 5 条，当前开发阶段严禁启动桌面应用、抢焦点或发送系统全局快捷键。Windows 原生端到端交互及视觉验收统一由 Codex 负责执行。
2. **原生遮罩窗口排除（`capture_region_below`）**：由 Qoder 在 `os-hal/src/platform/screen.rs` 与 `screenshot_window.rs` 中独占实现；本分支确保前端遮罩窗口在选区内达到 100% 全透且无边框残留，与 Qoder 的原生遮罩排除能力形成闭环双保险。

---

## 6. 提交信息

- **分支**：`codex/windows-scroll-overlay`
- **提交哈希**：`952c070e927d64e9692fbb59d3224c296b35f2d0`（短哈希：`952c070`）
