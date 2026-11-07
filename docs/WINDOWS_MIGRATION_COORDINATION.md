# Windows 并行迁移任务（2026-11-07）

集成负责人：Codex。四个外部编辑器智能体各自使用独立 Git worktree，禁止在主工作目录切换自己的任务分支或修改其他人的文件。集成、冲突处理、最终工作区测试和合并统一由 Codex 完成。

| 智能体 | 分支 | 工作目录（与主仓库同级） | 负责范围 |
| --- | --- | --- | --- |
| Cursor / Grok | `codex/windows-vault` | `YourAllIn1Helper-windows-vault` | 密码本 Windows Hello/系统保护的设备密钥槽，以及密码本自身的 Windows 原生会话/电源锁定观察者 |
| Antigravity / Gemini | `codex/windows-usage` | `YourAllIn1Helper-windows-usage` | OS HAL Windows 会话/睡眠/屏幕事件、可靠锁屏状态、默认输出设备的真实音频活动探测 |
| CodeBuddy / MiMo | `codex/windows-secure-services` | `YourAllIn1Helper-windows-secure-services` | Windows SecretStore、敏感剪贴板写入与序号所有权、GIF 剪贴板写入 |
| Zed / Copilot | `codex/windows-input-regions` | `YourAllIn1Helper-windows-input-regions` | Windows 粘贴目标记忆/恢复及物理快捷键、截图 UIAutomation 控件/父级区域查询 |

## 公共要求

- 先阅读 README、`docs/MODULE_ARCHITECTURE.md` 和本轮 `docs/testing/WINDOWS_STARTUP_2026-11-07.md`，据源码判断现状，历史规划不代表实现状态。
- 只在指定 worktree 编辑、运行测试及提交。不要合并其他分支、改写历史或向远程推送。
- 业务逻辑、配置、测试分离；新增代码有中文注释；生产路径必须使用真实 Windows API，不能以 Mock、空实现、固定布尔值冒充系统能力。
- 保留 macOS 的现有实现。非 Windows 的条件编译边界必须正确。
- 每人使用自己 worktree 下的 `target` 目录，编译并发最多 2；已有 Cargo 缓存由系统自然共享，避免四份构建争抢同一输出文件。
- `crates/os-hal/Cargo.toml` 的 Windows feature 可能被多人增补，只做必要增量，保留 windows-rs 的锁定版本；不要无故重写 Cargo.lock。冲突由集成负责人解决。
- 不编辑他人负责的模块；发现跨范围问题在自己的报告中列出。共享 Trait 需要扩展时保持向后兼容，并说明依赖。
- 敏感能力的测试仅使用虚构数据和本应用独立命名空间；普通单测不得读取用户真实剪贴板、选区或凭据，不得弹出认证框或修改外部应用窗口。真实桌面/硬件测试显式 ignored，单独报告。
- Windows Hello 认证必须与受系统保护的设备密钥访问绑定，不能先校验一个布尔值再读取无认证保护的 DKEK。硬件、策略或当前宿主不支持时返回明确不可用，保留主密码路径；不得把 PIN 验证、DPAPI 或普通凭据存储误称为生物识别等价保护。
- 对实际负责的模块补充有意义的回归单测，覆盖失败、取消、权限/能力缺失、清理和重复启停；执行相关 crate 测试及必要的 Windows 编译检查。
- 完成后提交自己的修改，写 `docs/testing/WINDOWS_<任务名>_2026-11-07.md`：实现、变更文件、测试命令/结果、未测硬件项、已知限制、最终提交哈希。不要虚报测试。
- 在现有聊天里回复完成状态。根目录不放临时输出，测试数据与日志放忽略目录。

## 文件职责边界

- Cursor：`crates/mod-password-vault/**`，必要的 `src-tauri/src/tauri_app.rs` 密码本 observer 装配；不修改 OS HAL secure、usage、input、regions 实现。
- Antigravity：`crates/os-hal/src/platform/usage/**` 及该能力的测试；不要修改密码本自身的 native observer。
- CodeBuddy：`crates/os-hal/src/platform/secure.rs` 及其拆分子文件、`crates/os-hal/src/secure.rs` 契约文档和相关测试；不要修改密码本 keystore。
- Zed：`crates/os-hal/src/platform/input.rs`、`regions.rs` 及新的 Windows 子实现、相应测试；必要时只改 NativeOsHal 的输入状态字段。不要修改翻译鼠标监听或其他功能行为。
