# 功能 01：增强截图与贴图 M3（滚动长截图 + GIF 录制）交接文档

- **执行者模型**：Gemini 3.8 Flash
- **工作区**：`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/antigravity-screenshot-m3`
- **分支**：`codex/feature-screenshot-m3`
- **起点基准**：`a721da6`（已合并本地 `master 53240cc`）
- **交付目标**：M3 滚动长截图（NCC 拼接、去重、固定首尾、低置信度安全停止、外置控制条、预览与复用 M1 导出）+ GIF 录制（NeuQuant 256 色量化流式编码、分辨率与帧率预设、独占式防覆盖创建与取消清理、退出线程等待）。

---

## 1. 提交记录

分批提交如下（正文均包含 `Agent: Antigravity` 与 `Model: Gemini 3.8 Flash`）：

1. **`6412c8e`** `feat(screenshot): 实现 M3 滚动长截图与 GIF 录制核心逻辑`
   - 新增 `crates/mod-screenshot/src/scroll/`：行特征与归一化互相关 (NCC) 纵向位移求取、固定头尾栏探测剔除与补回、重复帧与反向位移过滤、低置信度安全停止、长图与内存上限守护、后台采集与自动滚动线程；
   - 新增 `crates/mod-screenshot/src/gif/`：分辨率预设比例换算、NeuQuant 256 色量化流式编码器、后台录制循环、`create_new` 独占式时间戳文件生成、取消清理临时文件；
   - 依赖声明：引入 MIT 许可的 `gif = "0.14.2"` 与 `color_quant = "1.1.0"`，严格杜绝 AGPL-3.0 的 `gifski`；
   - 新增 `crates/mod-screenshot/tests/unit/{scroll,gif}.rs` 与 `tests/m3_lifecycle.rs`。
2. **`287a9c4`** `feat(host): 接入长截图与 GIF 控制窗口层级提升及 IPC 命令`
   - 扩展 `src-tauri/src/ipc/screenshot.rs`：追加 `screenshot_scroll_*`、`screenshot_gif_*` 命令；
   - 扩展 `src-tauri/src/screenshot_binary.rs`：追加 `screenshot_scroll_image` 原始像素流读取命令；
   - 扩展 `src-tauri/src/tauri_app.rs`：处理 `CONTROL_LABEL` 与 `PREVIEW_LABEL` 窗口生命周期，将控制条窗口安全提升至系统顶层（高于菜单栏与遮罩），注入事件监听转发；
   - 新增 `src-tauri/capabilities/screenshot-events.json` 权限。
3. **`6dd0573`** `feat(ui): 增加长截图与 GIF 控制条、预览弹窗、工具栏选项与三语字典`
   - 新增 `src-ui/src/components/ScreenshotControl.vue`（浮动控制条）与 `src-ui/src/components/ScreenshotPreview.vue`（无缝长图预览与导出）；
   - 在 `CaptureOverlay.vue` 遮罩工具栏追加「长截图」与「GIF」入口及 GIF 预设配置子栏；
   - 新增 `src-ui/src/utils/screenshot/{scroll,gif}.ts` 与前端单元测试；
   - 补齐 `zh-CN.json`、`en-US.json`、`ja-JP.json` 字典 `screenshot.scroll.*` 与 `screenshot.gif.*` 词条；
   - 在 `docs/testing/GUI_TEST_SCREENSHOT.md` 追加 `SHOT-M3-01`、`SHOT-M3-02` 用例（标记为「未执行」）。

---

## 2. 依赖与开源协议合规说明

- **严格规避 AGPL**：原计划可能涉及的 `gifski` 采用 AGPL-3.0 许可，与本项目的 MIT 协议存在冲突，**本项目未引入 `gifski`**。
- **引入合法依赖**：
  - `gif = "0.14.2"`：采用 MIT / Apache-2.0 双许可；用于流式 GIF 文件结构组织与帧编码；
  - `color_quant = "1.1.0"`：采用 MIT 许可；基于 NeuQuant 神经网络颜色量化算法，将每帧 RGBA 颜色压缩至高质量 256 色调色板。
- 图像缩放复用既有依赖 `image` 的 Lanczos3 / Triangle 高质量双线性与区域采样算法。

---

## 3. 核心机制与架构设计

### 3.1 滚动长截图 (`crates/mod-screenshot/src/scroll/`)
1. **控制条窗口物理隔离**：
   - 控制条窗口位于选区下边缘外部 `(cx, cy) = (sel.x + (sel.w - 340)/2, sel.y + sel.h + 12)`，若接近屏幕底部则自动翻转至选区顶部上方 `sel.y - 60`；
   - 窗口层级提升至 `NSMainMenuWindowLevel + 2`，确保即使被遮挡应用滚动也能浮动在最前，且**绝对不会被截图采入画面**。
2. **归一化互相关 (NCC) 与特征行搜索**：
   - 提取帧下部模板区域与新帧进行加权互相关计算；
   - 先通过行平均亮度与极差哈希做粗筛，再进行逐像素 NCC 相似度打分（相关度 $\ge 0.85$ 为有效匹配）；
   - 出现重复帧或 $\Delta y = 0$ 时判定为未滚动（`ZeroDisplacement`），不盲目重复追加画布高度；
   - 当置信度低于阈值时触发 `LowConfidence`，**安全停止并完整保留已有拼接结果**，如实向用户提示原因，杜绝交付错位扭曲的长图。
3. **固定头尾栏探测与重构**：
   - 探测顶部固定导航栏（行变化差值接近 0）与底部固定标签栏；
   - 拼接过程中仅保留内容活动区位移；停止时将已剥离的头部与底部导航栏重新复位，输出自然完整的页面长图。
4. **资源上限保护**：
   - 单张长图最大高度保护（默认 30,000 物理像素）；
   - 内存上限保护（默认 128 MB RGBA 像素缓冲）；达到上限时自动平滑停止。
5. **预览与复用 M1 导出**：
   - 完成后呼出独立预览窗口，支持无缝纵向滚动浏览；
   - 复用 M1 管道直接进行剪贴板复制、`create_new` 防覆盖另存为与多屏独立贴图。

### 3.2 GIF 录制 (`crates/mod-screenshot/src/gif/`)
1. **预设与尺寸计算**：
   - 提供「原画、1080p、720p、480p」及自定义宽高；
   - 保持长宽比，并自动对齐至偶数尺寸以保证编码器稳定。
2. **流式增量编码**：
   - 后台线程以设定帧率（10/15/24/30 FPS，对应 10/7/4/3 厘秒帧延迟）连续采集；
   - 每帧动态生成 NeuQuant 256 色全局/局部调色板并写入临时文件，内存占用恒定，不爆显存。
3. **独占创建与清理保护**：
   - 录制保存通过 `write_exclusive_gif` 采用 `OpenOptions::new().write(true).create_new(true)` 保证绝不覆盖已有文件；
   - 用户点击取消或按 Esc 时，立即删除临时半成品文件，不遗留孤儿垃圾。
4. **模块停用安全**：
   - 模块停用（`on_disable`）或退出时，设置原子取消信号并 `join` 等待录制线程与滚动线程安全退出，关闭对应窗口。

---

## 4. 接口与字典清单

### 4.1 IPC 命令与事件
- **长截图**：
  - `screenshot_scroll_start`: 启动滚动会话
  - `screenshot_scroll_stop`: 结束长截图并进入预览
  - `screenshot_scroll_cancel`: 取消长截图并释放
  - `screenshot_scroll_status`: 查询长截图运行状态
  - `screenshot_scroll_set_auto`: 开启/关闭注入模拟滚轮向下滚动
  - `screenshot_scroll_info`: 读取拼接图片尺寸与停止原因
  - `screenshot_scroll_export`: 导出长截图（copy/save/pin）
  - `screenshot_scroll_image`: 二进制通道读取拼接图片完整 RGBA
  - 事件：`screenshot://scroll-frame-stitched`
- **GIF 录制**：
  - `screenshot_gif_start`: 启动 GIF 录制
  - `screenshot_gif_stop`: 停止录制并保存最终 GIF
  - `screenshot_gif_cancel`: 取消录制并清理临时文件
  - `screenshot_gif_status`: 查询 GIF 录制时长、帧数与文件大小
  - 事件：`screenshot://gif-recording-stats`

### 4.2 字典键（追加无重构）
- `zh-CN.json`、`en-US.json`、`ja-JP.json` 均统一新增：
  - `screenshot.scroll.start`
  - `screenshot.scroll.scrolling`
  - `screenshot.scroll.height`
  - `screenshot.scroll.autoScroll`
  - `screenshot.scroll.stopAuto`
  - `screenshot.scroll.finish`
  - `screenshot.scroll.previewTitle`
  - `screenshot.scroll.lowConfidenceTip`
  - `screenshot.scroll.maxLengthTip`
  - `screenshot.scroll.copySuccess` / `saveSuccess` / `pinSuccess`
  - `screenshot.gif.start`
  - `screenshot.gif.frames`
  - `screenshot.gif.stop`
  - `screenshot.gif.record`
  - `screenshot.gif.previewTitle`
  - `screenshot.gif.savedSuccess`
  - `screenshot.gif.presetOriginal` / `preset1080p` / `preset720p` / `preset480p`

---

## 5. 真实测试验证结果

| 测试项 | 执行命令 | 结果 | 耗时/备注 |
| --- | --- | --- | --- |
| `mod-screenshot` 单元测试 | `cargo test -p mod-screenshot --offline` | **35 passed, 0 failed** | 0.24s |
| `mod-screenshot` M1 宿主生命周期 | `cargo test -p mod-screenshot --test lifecycle --offline` | **14 passed, 0 failed** | 4.60s |
| `mod-screenshot` M3 滚动与 GIF 生命周期 | `cargo test -p mod-screenshot --test m3_lifecycle --offline` | **3 passed, 0 failed** | 1.80s（真实 GIF 解码验证、取消临时文件清理验证、停用等待验证） |
| `all-in-one-host` 截图集成测试 | `cargo test -p all-in-one-host --test screenshot` | **3 passed, 0 failed** | 3.55s |
| 整体工作区全量测试 | `cargo test --workspace --all-targets` | **全部通过（0 错误）** | 覆盖全工作区所有模块与目标 |
| Rust 代码格式校验 | `cargo fmt --all -- --check` | **Clean（0 差异）** | 完全合规 |
| 前端多语与逻辑单元测试 | `pnpm --dir src-ui test` | **20 passed, 0 failed** | 覆盖三语字典 schema 校验、长截图与 GIF 工具格式化测试 |
| 前端生产打包与类型校验 | `pnpm --dir src-ui run build` | **0 errors (Success)** | `vue-tsc --noEmit && vite build` 编译成功 |

---

## 6. 未执行项与平台限制说明

1. **GUI 实体人工操作（SHOT-M3-01 / SHOT-M3-02）**：
   - 依据项目协作规范，开发者环境测试采用纯逻辑、合成图像位移与真实文件流解码进行全流程自动化测试；
   - 交互级用例已同步追加至 `docs/testing/GUI_TEST_SCREENSHOT.md`，状态如实标为「未执行」，留由协调者在第三轮集成验收时通过图形界面执行。
2. **自动滚动辅助功能权限**：
   - 在未授予 macOS「辅助功能」权限的环境下，自动滚动将如实回退为手动滚动，并向用户提示；长截图核心拼接完全支持用户手动平滑滚动。
3. **Windows 实机环境**：
   - 依赖注入的 `InputSimulator` 与 `ScreenCapturer` 抹平平台差异，但物理机多 DPI 交互需由协调者在 Windows 环境验证。
