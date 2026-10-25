# Translator M1 集成审查修复

- 基线：`9cca0d348772d80073921b066aac17e0052254ba`；仅在 translator 专用 worktree 修改。
- 实施：OpenAI Codex。原 M1 交接中的历史测试记录保留，本文件说明集成审查后的增量及边界。

## 已修复

1. Reqwest 原配置只有 JSON、没有 TLS 后端，官方 HTTPS 请求必然失败。启用系统 native-tls，更新 lock；新增自有 loopback TLS ClientHello 回归，未调用真实服务商。
2. macOS 选区文件按平台 cfg 编译，避免 Windows 引用 Apple framework/目标专用依赖；Windows 原生选区仍明确 Unsupported，能力元数据返回 false。修正 AX 不支持属性/未实现错误码，读取前检查权限，验证 CFString 类型并区分读取错误。
3. 同步快捷键先捕获并缓存原文/权限/Unsupported 状态，再开窗口。气泡注册监听后一次消费，已有气泡通过 selection-ready 事件消费新状态；不再聚焦后重新取词，也不从同步主线程裸调用 tokio::spawn。宿主把 AX 等待移到 blocking worker。
4. IPC 同步登记请求，无需 block_in_place/block_on；current-thread runtime 也可调用，无运行时则明确返回错误。请求创建、句柄登记、取消共用互斥，关闭/失焦销毁/停用都会取消；停用清空待消费原文。
5. 两个真实 Vue 脚本在 IPC request_id 回包前缓冲早到事件，回包后仅回放匹配请求；未发请求的视图不再接收另一视图的翻译。卸载中途异步注册的监听也及时解除。
6. 自定义服务 ID 不能覆盖内置服务或重复，避免重用官方 API Key 向任意自定义端点发送；删除配置会撤销旧 provider。仅解析 URL 主机识别 loopback，阻止路径/用户名/伪 localhost 域名绕过凭据检查；远程端点要求 HTTPS，不接受 URL 用户名/密码。
7. SSE 收到 [DONE] 立即结束，服务端保持连接也不会继续等待超时；服务端 error 或提前 EOF 明确失败。普通设置写入清洗后的配置。
8. API Key 仍仅本会话内存存储，UI 删除“内存加密”不实说明；自动复制、历史尚未实现，复选框禁用并注明，配置字段保留兼容。
9. 原硬编码 localhost:11434 测试改为不联网的能力断言；普通 os-hal 选区测试不再读取真实前台应用，原生取词留给显式 GUI 验收。

## 验证

所有 Cargo 构建使用本 worktree 的独立 `target`。

- `cargo test -p mod-translator --offline`：9 单元 + 7 自有 HTTP/TLS fixture + 2 生命周期，共 18 通过。
- `cargo test -p all-in-one-host --lib translator_ --offline`：3 通过，涵盖 IPC 停用门禁、current-thread runtime、无 runtime 明确报错。
- `cargo test -p os-hal --lib test_selection_extractor_contracts --offline`：1 通过（修正后的测试只读取 Mock 与能力元数据）。
- `cargo check -p all-in-one-host --all-targets --offline`：通过。
- `pnpm --dir src-ui test`：6 通过，含直接执行实际 Vue 组件脚本的事件乱序回归。
- `pnpm --dir src-ui run build`、`cargo fmt --all -- --check`、`git diff --check`：通过。
- 沙箱禁止绑定本地端口，fixture 经正规自动审批后在沙箱外执行。TLS 缺失包通过官方 Cargo registry 的 `cargo fetch --target aarch64-apple-darwin` 正规下载。

## 未冒充完成的边界

- 无真实 Google/DeepL/OpenAI API 调用，无真实用户密钥读取、无 GUI 操作。审查过程中曾运行原选区契约测试，其原生读取只得到 PermissionDenied/Unsupported/None 允许状态，没有文本返回；现已删除该自动读取路径。
- 真实 macOS GUI 焦点/快捷键/气泡行为仍需显式手工验收；未执行 Windows 交叉编译或 UIAutomation 实测。
- Windows UIAutomation 未实现；系统 Keychain/Credential Locker 未实现，仅会话凭据。popover 配置宽高尚未接入 WindowSpec。
- 原约定 M2：共享剪贴板租约后的模拟复制、全局鼠标拖拽钩子、国内翻译 provider。保持明确 Unsupported/待办，不伪造结果。
