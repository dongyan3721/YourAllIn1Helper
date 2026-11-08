# Windows 剪贴板候选窗焦点修复（2026-11-08）

- 分支 / 工作目录：`codex/windows-clipboard-focus` / `D:\YourAllIn1Helper-worktrees\focus`
- 基线：`a2c130c`（协调文档提交，其父为 `0cdada8`）
- 代码提交：`7ac74df`（本文档在其后的单独文档提交中加入，该提交哈希见会话回复）
- 执行者：Zed Agent / Claude Sonnet 5.5
- 范围声明：**没有启动桌面应用、没有 GUI 操作、没有发送全局按键**。真实桌面验收留给 Codex。

## 缺陷

WorkBuddy 等 VS Code 类 IDE 里用全局快捷键呼出剪贴板候选窗后，焦点仍在 IDE，上下键移动的是 IDE 光标，选不了候选项。

## 根因

本轮没有做 GUI 复现，以下是代码路径分析得出的结论，其中第 1 点是推断，需要 Codex 在 GUI 验收时印证。

1. **候选窗从未验证过自己真的拿到了前台。**
   - 弹窗只靠创建时的 `.focused(true)`，即 `WebviewWindowBuilder` 的创建期激活请求。创建之后没有任何一步检查前台窗口是不是它。
   - 在 IDE 里按热键时，Windows 前台锁可能拒绝激活新建的瞬态窗口。该窗口 `skip_taskbar` 且没有任务栏按钮，被拒绝后没有任何可见提示，前台仍是 IDE。
   - 同时 `on_shortcut` 里的 `open_window` 返回了成功，弹窗被登记为已打开。
2. **WebView 内容焦点不等于顶层窗口前台。**
   - Tauri 的 `WebviewWindow::set_focus` 只激活顶层 HWND，不会 `MoveFocus` 到 WebView2。
   - 前端原来只在 `onMounted` 里对搜索框 `focus()` 一次，之后的窗口激活不再处理。
   - 方向键监听挂在容器 `div` 上。WebView 内没有任何元素持有焦点（`activeElement` 为 `body`）时，按键到不了这个监听器。
3. **Tauri/tao 的 `set_focus` 在被拒绝时会注入 Alt 键。**
   - tao 0.37.2 的 `force_window_active` 在 `SetForegroundWindow` 失败后用 `SendInput` 发 Alt 来骗取前台权限。
   - 这个按键会发给当时的前台窗口，也就是 IDE（单独的 Alt 会激活 IDE 菜单栏）。这违反“不向错误窗口发送输入”，所以不能用 `set_focus()` 掩盖问题。
4. **粘贴目标的生命周期没有和弹窗会话绑定（目标记忆边界缺口）。**
   - `remember_paste_target` 只在打开弹窗时写入，成功粘贴时才被 `take`。
   - 弹窗被取消、失焦关闭、打开失败，或再次按热键关闭时，目标都会残留。
   - 随后从 Dashboard 点“粘贴”，`restore_and_paste` 会把前台抢回这个过期的 IDE 窗口并发送 Ctrl+V。

## 实现

### `src-tauri/src/clipboard_window.rs`
- 新增 `present_new_popup`（新建后）和 `focus_popup`（复用已有窗口时）。
- Windows：
  1. 取弹窗 HWND，按整数比较。`Cargo.lock` 中同时有 `windows 0.61.3` 与 `0.62.2`，Tauri 返回的 HWND 与本 crate 的 HWND 不保证是同一类型，`native_dialog.rs` 也是转成整数使用。
  2. 若弹窗不是前台，调用**一次** `SetForegroundWindow`。不用 Alt 注入，不用 `AttachThreadInput`，不无限重试。
  3. 用 `wait_until_foreground` 做有界等待：每 10ms 检查一次，上限 250ms，超时即返回失败。
  4. 确认弹窗是前台后，调用 `Webview::set_focus`，也就是 WebView2 的 `MoveFocus`，把键盘焦点交给页面。
  5. 失败时返回带诊断信息的错误，说明哪个 HWND、哪个进程保留了前台。
- 非 Windows：`focus_popup` 仍调用 `window.set_focus()`，`present_new_popup` 是空操作，行为与原来一致。macOS 的 `prepare_popup` 未改动。

### `src-tauri/src/tauri_app.rs`（只改 popup 分支）
- `create_or_focus_window` 的已有窗口分支：`clipboard-popup` 走 `focus_popup`，其他窗口仍走原来的 `set_focus`。
- 新建窗口分支末尾增加 `present_new_popup`。
- 两处失败时都 `window.destroy()` 并返回 `CoreError::WindowLifecycle`，不留下无法接收键盘的孤立弹窗，也不向窗口池登记。

### 粘贴目标生命周期（`os-hal` 与 `mod-clipboard`）
- `os-hal`：`InputSimulator` 新增 `forget_paste_target`，默认空实现。
  - `NativeOsHal` 在 Windows 清空已记忆的 `(HWND, pid)`。
  - macOS 保持空实现，保留原有的“重新激活上一个应用”行为。
- `mod-clipboard`：
  - `on_shortcut` 打开弹窗失败时，清除目标并返回原错误。
  - 再次按热键关闭弹窗时，无论销毁是否成功都清除目标。
  - `paste_item_with_prepare` 在没有活动弹窗会话时先清除过期目标，再继续。已有弹窗的粘贴流程不变。
  - 因此 Dashboard 的粘贴在没有目标时会明确报“没有已记忆的原粘贴目标”，而不会误粘到旧 IDE 窗口。
  - 这个判断不依赖失焦事件。失焦事件与“粘贴时先销毁弹窗”存在竞态，用它清除目标可能误伤正在进行的粘贴。

### `src-ui/src/components/ClipboardPopup.vue`（仅脚本和模板事件，CSS 未动）
- 键盘处理从容器 `@keydown` 改为在 `window` 上监听 `keydown`，`onBeforeUnmount` 时移除。WebView 内没有元素持有焦点时方向键也能用。
- 监听器先于历史加载登记，加载期间到达的按键不丢失。
- 处理 `Escape`、`ArrowUp`、`ArrowDown`，并新增 `Enter`。
  - `Enter` 在聚焦按钮时让位给按钮自身。
  - 搜索框的 `Enter` 已 `preventDefault`，不会重复粘贴。
  - `Escape` 在空列表时仍然关闭弹窗，保留原行为。
- 输入法组词期间（`isComposing`）的按键不被列表导航抢走。
- 窗口激活时（`focus` 事件）把焦点交回搜索框。
- 搜索词变化后高亮回到第一项，避免索引越界。
- 方向键移动后用 `scrollIntoView({ block: 'nearest' })` 让高亮项保持在可视区。

## 共享改动与依赖（交 Codex 集成）

- `crates/os-hal/src/traits.rs` 新增 trait 方法 `forget_paste_target`，有默认实现，所有现有实现者不必改。
- `crates/os-hal/src/platform/input.rs` 与 `platform/windows/input.rs` 增量改动，未全文件格式化。
- 没有新增 Cargo 依赖或 Windows feature。`src-tauri` 已启用的 `Win32_Foundation`、`Win32_UI_WindowsAndMessaging` 足够，`Cargo.lock` 未改。
- 没有碰 ClipboardPopup 的 CSS 或字体，没有碰 `ipc/kernel.rs`，没有碰截图相关的 `tauri_app.rs` 区域。
- `src-tauri/src/tauri_app.rs` 里原有的未用导入 `os_hal::ExternalWindowInfo`，以及 `screenshot_window.rs` 的未用 `Manager`，是已有 warning，非本次引入。

## 测试

环境：Windows，`jobs=1`，target 在 `D:\YourAllIn1Helper-worktrees\focus\target`。前端依赖用 `pnpm --dir src-ui install --frozen-lockfile --offline` 按原锁文件安装，没有升级，`node_modules` 被 gitignore。

| 命令 | 结果 |
| --- | --- |
| `cargo test -j 1 -p os-hal --lib windows::input` | 5 passed（含新增 `forgetting_target_clears_it_so_later_paste_is_refused`） |
| `cargo test -j 1 -p mod-clipboard` | 2 单元 + 6 集成（新增 2 个，另 1 个断言更新）全部通过 |
| `cargo test -j 1 -p all-in-one-host --lib clipboard_window` | 6 passed |
| `cargo test -j 1 -p all-in-one-host --lib` | 30 passed |
| `npm test`（`src-ui`，全部 10 个单测文件） | 80 passed |
| `node ... --test tests/unit/clipboard-popup.test.mjs` | 9 passed |
| `npx vue-tsc --noEmit`（`src-ui`） | 无输出，无类型错误 |
| 对本次改动文件执行 `rustfmt --check` | 通过 |

新增/调整的测试覆盖点：
- **有界等待**：已是前台时不等待；几次轮询后获得前台；被拒绝时在期限处停止，不会无限重试；超时常量保持短且有上限。
- **句柄判断**：空 HWND 永远不算前台；拒绝信息包含保留前台的窗口和进程。
- **目标记忆**：清除后 `take_remembered_target` 返回 `NotFound`。
  - 打开失败时清除目标，且窗口池里没有残留。
  - 再次按热键关闭时清除目标。
  - 无弹窗会话的粘贴先清除目标，再关闭并发送。
  - 原有“弹窗权限被拒/剪贴板写入失败时保留弹窗、不发送粘贴”的用例仍通过。
- **前端键盘**：
  - 焦点在 `body` 时方向键仍能选择，并且循环。
  - `Enter` 只粘贴一次。
  - 聚焦按钮或输入法组词时不被劫持。
  - 空列表 `Esc` 仍销毁弹窗。
  - 窗口激活后焦点回到搜索框。
  - 搜索变化重置高亮。
  - 卸载后移除监听。

普通测试没有读写真实剪贴板，没有调用 `SendInput`，没有创建窗口。`SetForegroundWindow` 路径和 WebView2 `MoveFocus` 只能在真实桌面验证。

## 未测项与已知边界

以下**都没有在真实 Windows 桌面上验证，不能记作通过**：

1. 在 WorkBuddy、VS Code、Cursor 等 IDE 内按 `Ctrl+Shift+V` 后：
   - 候选窗是否成为前台；
   - 上下键是否移动高亮；
   - 回车是否粘贴到原 IDE 编辑器。
2. 根因第 1 点（IDE 场景下前台锁拒绝激活）是推断。Codex 验收时若仍失败，查看日志里 `Global shortcut action failed` 的错误：
   - 失败信息包含保留前台的进程号和 HWND，用来区分“前台权限被拒”和“WebView 内焦点”两类问题。
   - 若 IDE 场景下出现“已取消呼出”，说明 Windows 确实拒绝了前台。当前选择是显式失败，没有用 Alt 注入或线程输入绑定绕过。是否需要放宽，需要用户决定。
3. 250ms 等待期间，如果在主线程调用，会短暂阻塞消息循环。仅在被拒绝的失败路径上才会等满。
4. 失败时只记录日志并销毁弹窗，用户界面没有可见提示。若验收发现需要托盘或通知提示，需要另行设计。
5. 混合 DPI、副屏、最小化 IDE、管理员权限的 IDE（UIPI 会阻止低权限进程向其发送输入）未覆盖。
6. 候选窗复用路径（窗口已存在于 Tauri 但不在窗口池里）只是代码路径对称处理，没有专项测试。
7. macOS 路径保留，但没有在 macOS 上编译或运行。
8. `forget_paste_target` 在 macOS 上为空实现。macOS 的 Dashboard 粘贴仍会重新激活旧目标，这是保留下来的既有行为，不是本次修复范围。
9. 原有 `cargo fmt --check` 在 `os-hal/tests/secure_services.rs` 和 `src-tauri/src/lib.rs` 里报告的格式差异不是本次引入，没有处理。

## 提交

| 提交 | 内容 |
| --- | --- |
| `7ac74df` | 源码与测试：Windows 剪贴板候选窗前台与键盘焦点修复 |
| 文档提交 | 本文件，哈希见会话回复 |

未合并、未推送，没有修改 C 盘主目录。
