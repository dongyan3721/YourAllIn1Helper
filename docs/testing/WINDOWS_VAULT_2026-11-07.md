# Windows 密码本设备密钥与会话锁定（2026-11-07）

分支 `codex/windows-vault`，工作目录 `YourAllIn1Helper-windows-vault`，基线 `c7c6a4d`。没有修改 os-hal 的 secure、usage、input、regions，没有 push，没有合并其他分支。

## 安全边界

设备密钥不是“先得到 Windows Hello 的 `Verified`，再从 DPAPI 或凭据管理器读出 DKEK”。

- 后端 ID：`windows_hello_ngc_rsa_oaep_v1`。
- DKEK 用 Microsoft Passport Key Storage Provider 里不可导出的 RSA-2048 公钥做 OAEP-SHA256 包装。密文放在当前用户 `LocalAppData\com.allinone.desktop\vault-hello\v1\<slot>.oaep`，数据库的 `os_protected_dkek` 仍保持 NULL。
- 解开 DKEK 的路径是 `NCryptDecrypt`。调用前设置官方属性 `PinCacheIsGestureRequired`，并带上 `NCRYPT_NO_CACHED_PASSWORD`。
- 登记时用 `NCRYPT_SILENT_FLAG` 解密一段随机探测值。只有原始 HRESULT 精确等于 `NTE_SILENT_CONTEXT`（`0x80090022`）才保存 DKEK。静默解密成功，或返回 `NTE_BAD_FLAGS`、`NTE_NOT_SUPPORTED` 等其他错误，都删除未完成的密钥并报告不可用。
- `UserConsentVerifier.CheckAvailabilityAsync` 只判断 Hello 是否已配置，不参与释放密钥。没有调用验证请求，也没有 `CryptProtectData`、`CredWrite` 或 DPAPI-NG。
- Windows Hello 手势包含人脸、指纹或 PIN。SDK 没有等价于 macOS `biometryCurrentSet` 的标志。界面在 `device_key_kind = windows_hello_gesture` 时使用 Windows Hello 文案，macOS 仍写 Touch ID。
- 提供程序、算法、导出策略读失败、手势属性被拒绝，或当前用户没有 LocalAppData 时，设备槽不可用，主密码继续有效。不把这些情况伪造成功，也不降级到 DPAPI。

## 锁屏观察者

密码本自己的隐藏窗口调用 `WTSRegisterSessionNotification(NOTIFY_FOR_THIS_SESSION)` 和 `RegisterSuspendResumeNotification`。锁屏、控制台断开、远程断开、注销、查询结束会话和 `PBT_APMSUSPEND` 会锁定。解锁和唤醒不自动打开密码本。登记失败会拆除半成品窗口，不留下空观察者。`src-tauri/src/tauri_app.rs` 在 Windows 和 macOS 上装配同一个启停入口。

## 变更文件

- `crates/mod-password-vault/src/windows_hello.rs`
- `crates/mod-password-vault/src/hello_store.rs`
- `crates/mod-password-vault/src/native/windows.rs`
- `crates/mod-password-vault/src/keystore.rs`、`service.rs`、`model.rs`、`lib.rs`、`native/mod.rs`
- `crates/mod-password-vault/tests/unit/hello_store.rs`、`windows_hello.rs`、`windows_observer.rs`
- `src-tauri/src/tauri_app.rs` 的密码本 observer 装配
- 密码本界面的 `device_key_kind` 文案
- `Cargo.lock` 只为密码本增加已有的 `windows 0.62.2` 和 `windows-future 0.3.2`

## 命令和结果

目标目录由本会话的 `CARGO_TARGET_DIR` 指定，是独立缓存，不是主仓库，也不是另外三个 worktree：

`C:\Users\xcxix\AppData\Local\Temp\cursor-sandbox-cache\f79565b907bc14ba3c612fd0bf3c15cb\cargo-target`

| 命令 | 结果 |
| --- | --- |
| `cargo test -p mod-password-vault --offline --jobs 2 --lib -- --test-threads=8` | 28 通过，0 失败，1 忽略 |
| 同一测试中的能力探测输出 | `windows hello gate: configured` |
| `cargo check -p all-in-one-host --offline --jobs 2` | 通过，exit 0，耗时 3 分 27 秒 |

`configured` 只表示 Passport 提供程序接受 RSA 查询，且 `CheckAvailability` 返回已配置。没有创建 NGC 密钥，没有执行会弹出认证框的 `NCryptFinalizeKey` 或非静默 `NCryptDecrypt`。

被忽略的测试是 `hardware_windows_hello_roundtrip_is_not_run_automatically`。它是空的，避免普通测试或 `--ignored` 误触真实认证框。

`src-ui/node_modules` 不在这个 worktree 里，没有运行前端测试。os-hal 的未使用项和不可达表达式警告来自基线，本分支没有改那些文件。

## 硬件未测

- 真实创建并 Finalize NGC 密钥，以及系统认证框的成功、取消、失败。
- 静默探测在本机是否真的返回 `NTE_SILENT_CONTEXT`。如果不是，登记会失败并保留主密码。
- NGC 是否接受 OAEP-SHA256 和 `NCRYPT_NO_CACHED_PASSWORD`。不接受时不降级到 PKCS#1 v1.5 或 DPAPI。
- 真实密钥的 Export Policy 是否可读，以及读失败时的登记拒绝。
- 用真实锁屏、注销、睡眠验证观察者。单测只向本进程隐藏窗口发送消息。
- Windows Hello PIN、指纹、人脸的实际因子，以及 PIN 变更后旧密钥是否仍可解。这不能当成 `biometryCurrentSet` 已验收。

## 提交哈希

实现与本记录在同一提交。最终哈希以该提交的 `git rev-parse HEAD` 为准，补记见下一节。

最终提交哈希：`PENDING_COMMIT`
