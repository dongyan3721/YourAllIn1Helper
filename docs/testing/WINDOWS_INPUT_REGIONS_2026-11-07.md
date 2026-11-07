# Windows 输入目标与截图区域验证（2026-11-07）

## 实现

- Windows 快捷键路径记录外部前台窗口的 HWND/PID。每次记忆操作先清空旧目标；前台为本应用时允许打开面板但不设置目标。无目标、目标已退出、HWND 被重用、目标变为本应用或恢复后前台不匹配时，粘贴明确失败。
- 关闭瞬态面板后仅调用受 Windows 前台限制约束的 `SetForegroundWindow`，确认原 HWND/PID 实际成为前台并再次校验后，才使用 `SendInput` 发送物理 Ctrl+V。Ctrl+C 仅发送给调用时仍匹配的外部前台窗口，不擅自激活其他窗口。
- Windows 区域查询在独立 COM 工作线程内启用每显示器 DPI 感知，使用 Win32 前台 Z 序定位外部 HWND，在该窗口 UIAutomation Control View 内按光标坐标搜索，并向上读取实际控件父链。输出控件、容器和顶层窗口的父子 ID；将控件边界裁剪到窗口边界，保留负坐标和物理像素。
- UIAutomation 遍历只进入命中窗口，深度限制 32、访问节点上限 1024；父链使用 UIA `CompareElements` 比较，并在整段父链遍历期间持有已比较 COM 接口引用，避免代理地址复用造成误判。树遍历器通过原始 vtable 保留 HRESULT 与空接口指针的区别：成功但空表示无子项/链尾，失败 HRESULT 才作为错误；失败时不会构造空接口。不可用的 UIA provider 会返回明确错误，不伪造控件结果。

## 变更文件

- `crates/os-hal/src/platform/input.rs`
- `crates/os-hal/src/platform/regions.rs`
- `crates/os-hal/src/platform/mod.rs`
- `crates/os-hal/src/platform/windows/mod.rs`
- `crates/os-hal/src/platform/windows/input.rs`
- `crates/os-hal/src/platform/windows/regions.rs`
- `crates/os-hal/tests/unit/windows_input.rs`
- `crates/os-hal/tests/unit/windows_regions.rs`
- `crates/os-hal/tests/capabilities.rs`
- `crates/os-hal/Cargo.toml`（增加 `Win32_UI_HiDpi` feature）

## 测试与边界

- `cargo fmt --manifest-path crates/os-hal/Cargo.toml -- --check`：通过。
- `cargo test -p os-hal --locked --all-targets`：通过；lib 11 passed、0 failed、1 ignored；capabilities 1 passed、0 failed、1 ignored；secure_services 3 passed。新增边界测试覆盖成功 HRESULT + 空接口、失败 HRESULT、成功接口、焦点目标选择/失败、父链裁剪与去重、多屏负坐标几何。
- `cargo check -p os-hal --locked --target x86_64-pc-windows-msvc`：通过。
- 构建使用本 worktree 的默认 `target`，`.cargo/config.toml` 限制并发为 2。
- 新增普通单测只使用虚构 HWND/PID 与矩形数据，不读取真实选区、不发全局按键、不操作其他应用窗口。
- `windows_ui_hierarchy_reports_real_chain_or_unsupported_provider` 是显式 ignored 的只读桌面测试；本轮未运行真实跨应用粘贴、Ctrl+C/V 或真实 UIAutomation 查询。
- UIAutomation provider 调用是同步 COM 调用。独立工作线程、COM RAII、深度/节点上限可控制正常遍历资源，但不能取消一个自身不响应的第三方 provider；此时工作线程及同步调用可能阻塞。本实现不声称覆盖所有应用的 UIA provider。
- 父链身份比较依赖 provider 对 `CompareElements` 的实现；被比较的接口引用在该次链遍历期间保持存活，不保留跨查询身份。
- 物理快捷键遵守 Windows 前台焦点限制；若系统拒绝激活目标则报告错误并取消输入。未尝试绕过前台限制。

## 提交

- 功能实现提交哈希：`b0e90ea135f9d425a11f05a67a26e64757a1cb24`。
