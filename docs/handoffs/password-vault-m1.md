# 功能 10：密码本 M1 实施交接

- 日期：2026-10-25
- 执行：WorkBuddy / Kimi K3.1 Pro
- 工作区：`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/workbuddy-password-vault`
- 分支：`codex/feature-password-vault`
- 范围：M1 主密码闭环、真实 Host/IPC/`/vault` 接线；只使用虚构测试数据。
- 状态：实现与自动化回归完成，**等待 M1 安全审查**；未 merge、未 push，未执行真实 GUI 验收。

## 1. 提交

| 提交 | 内容 |
| --- | --- |
| `a5c765aebbb297875668392b8bd258dd2fd76be6` | Argon2id/AES-GCM 核心、SQLite v1、主密码服务与单元测试 |
| `bbc9e9d41afb3b20c963adfddbca3f404ed34011` | 原生锁定通知适配、前端短驻留与密码本面板 |
| `82eca81c44f231134bfcdd013ae544462d7d78a7` | `SecretString`/`Zeroizing` 加固、测试稳定化 |
| `d1ef2eec315d32259f6cf7985555cc9e4f5c40ef` | Host 模块装配、异步 IPC、原生生命周期、`/vault` 与三语字典 |
| `d67680b733325fe6f1baf652cdfab70788fb61a0` | 升级集成测试纳入已挂载的密码本模块 |

每次提交正文均记录 `Agent: WorkBuddy` 与 `Model: Kimi K3.1 Pro`。本交接及 GUI 用例在后续文档提交中保存。

## 2. 冻结格式与依赖

### 主密钥与字段信封

- 每个密码本由 OS CSPRNG 生成随机 256 位 DEK；主密码不直接加密条目。
- 唯一接受的 KDF profile：`argon2id-v19-m65536-t3-p4`，Argon2 v19、16 字节盐、32 字节 KEK。
- 主槽：12 字节随机包装 nonce；包装结果为 48 字节（32 字节 DEK + 16 字节 GCM tag）。
- 主包装 AAD：`AIOVMW1 || crypto_version:u16be || vault_id:16 || canonical_kdf_profile`。
- 七个用户字段分别使用 AES-256-GCM、独立 12 字节随机 nonce 与 16 字节 tag。
- 字段 AAD 固定 50 字节：`AIOVDF1 || crypto_version:u16be || vault_id:16 || entry_id:16 || field_id:u8 || record_version:u64be`。
- 当前会话记录已用 nonce 并拒绝重复；`UNIQUE(entry_id, nonce)` 只是额外数据库约束，**不被当作全 DEK 历史唯一性的数学证明**。

锁定 RustCrypto/随机/清零版本：`argon2 = 0.5.3`、`aes-gcm = 0.10.3`、`getrandom = 0.3.4`、`zeroize = 1.9.1`。macOS 通知适配使用 `objc2 = 0.6.5`、`block2 = 0.6.2`、`objc2-app-kit = 0.3.2`、`objc2-foundation = 0.3.2`。此前已用 `cargo fetch --locked` 从官方源补齐依赖，最终测试均离线执行。

### SQLite v1 与迁移

`password_vault` 模块只新增空表迁移，不改旧模块表：

- `vault_meta`：vault ID、格式/KDF 元数据、盐与包装后的 DEK。
- `password_vault_entries`：条目 ID、vault ID、记录版本、创建/更新时间、归档位。
- `password_vault_fields`：七字段密文、nonce、tag、字段/记录/格式版本。
- `vault_device_slots`：为后续真实设备钥匙保护预留；M1 不读写、不启用。

title、URL、category、username、password、TOTP secret、notes 全部加密。明文仅保留 ID、版本、时间戳、归档位、密文与 KDF/格式元数据。搜索需要解锁后解密记录，匹配 title、username、URL、category、notes；不匹配 password/TOTP。

启动校验包括迁移 checksum、未来格式拒绝、`PRAGMA integrity_check` 和 `PRAGMA foreign_key_check`。损坏、AEAD 失败、字段/AAD 交换、结构或版本异常 fail closed，不返回秘密并锁定当前会话；不自动修库、不自动导入。完整更新在单事务内认证旧记录、检查 `expected_version`、递增版本、重加密全部七字段并提交。

## 3. 服务、生命周期与 IPC

### 状态与并发

- 状态机：未初始化 / 锁定 / 解锁中 / 已解锁。
- 同一服务解锁 KDF 并发为 1；Argon2、SQLite 与加解密经 `spawn_blocking` 离开 UI/AppKit 主线程。
- 每次解锁尝试与锁定操作推进 epoch；手动锁定、页面离开或模块停用后，迟到 KDF/敏感结果不得恢复旧会话。
- 后端密钥由 `Zeroizing` 管理；用户字段使用不实现 `Debug`/`Clone` 的 `SecretString`。
- 主密码修改会验证旧包装、用新盐和新 KEK 重包同一 DEK，成功后立即强制重锁。
- 密码生成器实际产生秘密前也检查解锁状态；非法策略可在不接触秘密的情况下直接拒绝。

### 锁定入口

- 复用 `ActiveAppWatcher::system_idle_seconds()` 与 `is_screen_locked()`，只读采样，不通过 IPC 或轮询刷新系统空闲。
- 真实系统空闲达到 180 秒、锁屏、能力查询失败均锁定。
- 后台每秒采样；采样间隔超过 3 秒按疑似睡眠保守锁定。CPU 饱和也可能触发此保守路径，不代表精确睡眠测量。
- macOS 已接入 NSWorkspace 睡眠/显示器睡眠/会话离开及 `com.apple.screenIsLocked` 通知；观察者在 AppKit 主线程成对注册/销毁。
- 主面板关闭、前端 `pagehide`/窗口隐藏/组件卸载、模块停用、应用退出均触发后端锁定并清理前端状态。
- `vault://locked` 只发送 epoch 和 reason，不携带密码、DEK 或条文明文。

### 真实 Host 路由

`src-tauri/src/modules.rs` 注册同一个 `Arc<PasswordVaultModule>`；生产 Tauri `ipc` 对 `vault_*` 使用专用异步入口。每个请求先经 `with_enabled("password_vault", …)`，服务内部再次检查 accepting/locked/epoch。旧同步 `invoke_ipc` 不执行密码本命令，避免误把 KDF 放入同步主线程调用链。

已实现命令：

- `vault_get_status`、`vault_initialize`、`vault_unlock_master_password`、`vault_lock`。
- `vault_list_entries`、`vault_get_entry`、`vault_upsert_entry`、`vault_delete_entry`。
- `vault_reveal_password`、`vault_generate_password`、`vault_change_master_password`。

未提供复制或生物识别命令；相关未知命令返回 M1 禁用错误。复制按钮和 Touch ID / Windows Hello 按钮在 UI 中禁用并说明原因。

### 配置边界

共享配置已有 `password_vault.biometric_enabled`、`auto_lock_secs`、`clear_clipboard_after_secs`。M1 **不消费这些开关**：自动锁定固定为 180 秒，复制始终禁用，生物识别始终禁用。即使共享配置默认 `biometric_enabled = true`，也不会触发生物识别、系统认证弹窗或 Keychain/DPAPI 写入。

## 4. 前端与公共文件

- Dashboard 新增 `/vault` 页签与路由，只有模块启用时显示。
- `VaultPanel.vue` 实现初始化、解锁、CRUD、搜索、揭示、生成器与主密码修改。
- 揭示值和生成结果最长驻留 15 秒；锁定、迟到 IPC、卸载和页面离开时清空列表、表单、主密码及生成/揭示值。
- 秘密不进入 localStorage、sessionStorage、路由或事件广播。显式 IPC 仍会产生 JS/WebView 字符串副本。
- 三语共享字典只新增 `vault.*`；组件临时字典改为共享 `t('vault.*')` 适配。
- GUI 验收清单：`docs/testing/GUI_TEST_PASSWORD_VAULT.md`，所有未实际执行项均标记“未执行”。

修改的公共/Host 文件：`Cargo.lock`、`src-tauri/Cargo.toml`、`src-tauri/src/lib.rs`、`src-tauri/src/modules.rs`、`src-tauri/src/ipc/mod.rs`、`src-tauri/src/tauri_app.rs`、`src-ui/src/components/Dashboard.vue`、三语 locale JSON，以及相关 Host 集成断言。未修改共享 EventBus、模块 02 或其他成员目录。

## 5. 三层证据

### A. 单元与集成测试：已执行

- `cargo test -p mod-password-vault --offline --all-targets`：17 通过，0 失败。
- `cargo test -p all-in-one-host --test modules vault_async_ipc_enforces_lock_and_module_switch --offline`：1 通过，0 失败。
- `cargo test --workspace --offline --all-targets`：**195 通过，0 失败，4 ignored**。
- `cargo fmt --all -- --check`：通过。
- `cargo clippy -p mod-password-vault --no-deps --all-targets --offline -- -D warnings`：通过。
- `pnpm --dir src-ui test`：22 通过，0 失败。
- `pnpm --dir src-ui run build`（含 `vue-tsc --noEmit`）：通过；仅有既有 chunk 大小提示。重复构建时环境拦截 Vite 清空 62 个旧产物，随后以 `vite build --emptyOutDir false` 不清空目录复核通过，未绕过删除保护或修改构建配置。

密码本测试覆盖独立 PHC 向量、错误密码、篡改、AAD 交换、重复 nonce、CSPRNG 失败、CRUD/搜索、版本冲突、主密码重包、迟到解锁、锁定/停用/重启与迁移隔离。Host 测试另覆盖锁定态 IPC 和模块开关拒绝。

4 个 ignored 均保持未执行：两个需要真实桌面会话，一个会写系统钥匙串专用测试项，一个会覆盖真实系统剪贴板。此前曾观察到既有 `mod-script-runner` 并行测试的异步事件竞态，单例及最终完整工作区回归通过；本轮未修改该模块，也不宣称已消除其潜在竞态。

### B. 原生编译/接线：已执行，实机通知未验收

- macOS Host 与原生观察者已随 `cargo check`、完整 `cargo test --workspace --all-targets` 编译。
- 主线程注册/销毁、窗口关闭、模块开关和退出清理已接线。
- **未启动真实应用做锁屏、睡眠、窗口关闭或退出的人工验收**；非 macOS 原生电源/会话观察器仍未实现，不伪报完成。

### C. GUI：未执行

- 未打开真实密码本页面，未进行 180 秒等待、锁屏/睡眠或三语人工检查。
- 未读取任何现有密码、浏览器凭据或个人数据库。
- 未触发 Touch ID / Windows Hello，未写系统钥匙串，未修改真实剪贴板。
- 协调者须按 `GUI_TEST_PASSWORD_VAULT.md` 在隔离数据目录中验收，不能用上述自动化结果替代 GUI 结论。

## 6. 明确安全边界与后续审查点

1. 不承诺全进程物理清零；不承诺 JS 字符串原地擦除、Serde JSON/WebView 副本、编译器临时副本、AES 扩展密钥或 FFI 缓冲绝对擦除。
2. nonce 防重是当前实现与防御性检查，不声称数学上证明整个 DEK 生命周期绝无随机碰撞。
3. SQLite 明文元数据没有独立防伪机制；不承诺整记录回滚防御、交换攻击的全域检测或恶意磁盘完全可信。
4. 沿用共享 SQLite WAL/NORMAL 策略，不声称 FULL 级断电持久性；主密码/条目写入需结合后续断电与故障注入审查。
5. 列表/搜索在解锁后产生明文 Rust 与 JS 副本；长驻留编辑表单的副本清零受 WebView 能力限制，15 秒只针对揭示值和生成结果。
6. 主密码强度只做长度/非空白基础检查，没有泄露口令库或强度评分；恢复密钥、自动备份与导入不属于 M1。
7. 共享剪贴板租约虽已由协调者实现，本功能未接入；M1 不复制秘密，也不声称 30 秒条件清除已经过本模块验收。
8. 生物识别真实设备钥匙保护属于 M2；M1 不把认证布尔值当 DEK，不读取/写入设备槽。
9. 未做独立安全审计、侧信道/内存取证、恶意数据库全面模糊测试、断电测试或跨平台实机验收。生产使用前必须完成审查。

等待 M1 安全审查。
