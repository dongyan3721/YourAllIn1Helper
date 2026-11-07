# Windows Hello 与导航栏修复（2026-11-08）

基于 `master` 的 `cf476e4`，修复分支 `codex/fix-windows-hello-navigation`。开始时 Cursor 留有未提交修改；原始 diff 已备份到忽略目录 `before-codex-hello-2026-11-08.patch`。本轮保留其他模块改动，只整理密码本 Hello 路径和主导航。

## 已定位的问题

1. Passport KSP 的持久密钥名需要当前用户作用域的路径。原先使用任意别名，创建返回 `NTE_INVALID_PARAMETER`。改为 `SID//com.allinone.desktop/vault/{slot_uuid}` 后，本机未 finalize 的真实创建与策略设置测试通过。
2. 本机 `webauthn.dll` 版本为 `10.0.26100.9949`。直接 Passport OAEP-SHA256（有/无 label）、OAEP-SHA1（无 label）的长度查询和实际加密均返回 `0x80090027`；软件 BCrypt OAEP-SHA256 加密后交给 Passport 静默解密，也返回该错误。PKCS#1 公钥加密成功，要求新手势后的静默解密返回 `0x80090022`。这些结论来自真实 KSP 探针，不能用软件 RSA 的支持情况推断 Passport 支持。
3. Cursor 的 `HelloPromptWindow` 主动创建、显示并置前额外窗口，且真实 dashboard HWND 未传入认证路径。该窗口及依赖它的 WinRT/WebAuthn 回退已移除。
4. `SIG1` 回退将用于派生 AES 解密密钥的签名与密文一起落盘，不能作为秘密保护。`SIG1` 和未完成的 `PRF1` 格式均拒绝解锁，不保留“认证布尔值通过后读密钥”的回退。
5. 功能导航位于带 `-webkit-app-region: drag` 的标题栏内。整条导航增加 `no-drag`，包含按钮外的横向滚动条区域。

## 最终行为与边界

- 新设备槽使用 Passport 的不可导出 RSA-2048 私钥，随机 32 字节 DKEK 使用本机支持的 RSAES-PKCS1-v1_5 包装。不是用于网络服务的通用解密接口。Windows Hello 的私钥访问控制、每次手势要求及静默反证仍必须成立；不回退 DPAPI、普通凭据存储或签名派生。
- 本地信封 v3 明确表示 PKCS#1；旧 v1 仍按 OAEP 解释，v2 试验格式拒绝。保留 `.oaep` 文件扩展名和旧数据库 backend 标签以兼容槽位查找，实际算法由信封版本决定，不能在读取失败时猜测填充或自动降级。主密码格式与条目密文不变。
- `NgcCacheType` / 旧属性名是 Passport 提供程序的兼容属性，不是跨提供程序的公开保证。设置/读回不满足、私钥可导出/可导入、用途异常，或静默解密实际返回明文，均拒绝登记。提供程序变化后应重新做原生验收。
- 交互操作由桌面 IPC 绑定现有 dashboard HWND，并验证 HWND 属于本进程；不创建额外启动窗口。单独的手工硬件测试可使用系统无 owner 的认证 UI，不代表产品需要辅助窗口。
- 解密直接提供模长缓冲，避免“查询长度 + 实际解密”造成多次私钥操作。失败回滚使用静默删除，失败时释放句柄，不再为了回滚弹出认证。
- Windows Hello 可以使用人脸、指纹或 PIN；不能宣称等同 macOS `biometryCurrentSet`。旧试验槽应通过主密码解锁后重新登记；不会批量删除用户已有系统通行密钥。

## 验证记录

日志位于 `.runtime_data/windows-validation/`：

| 项目 | 结果 / 日志 |
| --- | --- |
| Rust 全工作区、全部 target | 302 passed、0 failed、16 ignored；`codex-hello-workspace-final-2026-11-08.log` |
| 密码本单元测试 | 34 passed、0 failed、3 ignored；包含版本化信封、试验格式拒绝、用户/应用/槽位隔离和非法 owner；`codex-hello-unit-final-2026-11-08.log` |
| 前端单元测试 | 73 passed；`codex-navigation-tests-2026-11-08.log` |
| TypeScript | 通过；`codex-typescript-2026-11-08.log` |
| Windows 调试版打包 | 通过；`codex-hello-desktop-final-2026-11-08.log` |
| 原生创建参数、RSA 填充探针 | 通过；`codex-hello-native-parameters-2026-11-08.log`、`codex-hello-padding-bcrypt-2026-11-08.log` |
| CLI 烟测 | 5 步通过，退出码 0；`codex-hello-cli-smoke-2026-11-08.log` |
| Hello 完整硬件往返 | **通过**：用户亲自完成系统认证；隔离槽登记、两次解密均返回预期虚构 DKEK，撤销后不能读取；退出码 0。`codex-hello-hardware-pkcs1-2026-11-08.log` |
| 桌面 UI 烟测 | PASS，退出码 0；`codex-hello-ui-smoke-2026-11-08.log`。stderr 有一条关闭期间的无效 HWND PostMessage 警告，未导致烟测失败 |
| 导航滚动条鼠标拖动 | 在 1040 宽的真实 WebView2 主窗口中，拖动底部横条后右侧“09 窗口置顶”可见，再反向拖回总览；窗口位置未变 |
| 正常数据目录中的主界面流程 | **用户确认通过**：重新启用 Hello、锁定后 Hello 解锁成功，没有额外小窗口，也没有密钥存储失败。最终普通实例已启动 |

早期 OAEP 完整测试在加密处失败，日志保留。一次需交互的能力诊断等待过久后结束进程，结果记为未完成，不能当成认证失败或成功。后续使用静默、公钥专用探针确认参数支持，不靠反复弹窗试错。

## 手动复测

1. 从最终产物启动，主密码解锁测试密码本并启用 Windows Hello；确认仅出现系统认证窗口。
2. 锁定后用 Hello 解锁，立即再次锁定、再解锁；每次应重新认证。取消后保持锁定，主密码仍可用。
3. 正常退出、重新启动并再次用 Hello 解锁；禁用后不再提供设备密钥解锁。不要向测试密码本放入真实秘密。
4. 主窗口保持原尺寸，拖动功能导航的横向滚动条，确认最右侧功能可见且窗口本身不移动。

参考：[NCryptEncrypt](https://learn.microsoft.com/en-us/windows/win32/api/ncrypt/nf-ncrypt-ncryptencrypt)、[NCryptDecrypt](https://learn.microsoft.com/en-us/windows/win32/api/ncrypt/nf-ncrypt-ncryptdecrypt)、[NCryptDeleteKey](https://learn.microsoft.com/en-us/windows/win32/api/ncrypt/nf-ncrypt-ncryptdeletekey)。API 所列填充不代表每个 KSP 都实现；具体结果以上述本机探针为准。
