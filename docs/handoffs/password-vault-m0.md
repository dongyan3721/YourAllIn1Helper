# HANDOFF — 功能 10 生物识别加密密码本 M0

## 状态与提交

- 当前状态：**等待 Codex 设计审查；密码本未实现、未挂载、未验收**。
- 实际执行智能体：WorkBuddy。
- 实际执行模型：Kimi K3.1 Pro。
- 专用 worktree：`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/workbuddy-password-vault`
- 分支：`codex/feature-password-vault`
- 基线：`4689c49fc162eb4039b2a900d38d6f904c199a8d`
- M0 设计提交：`ac48dc672c9fb8d9d0fe1a3912140cc26d1b2ad0`（`docs(vault): add M0 security design`）
- 设计文档：[`docs/design/password-vault-m0.md`](../design/password-vault-m0.md)
- 本交接文档为后续独立提交；未 merge、未 push，未修改主目录、旧媒体分支或其他成员目录。

## 推荐方案

1. **密钥分层**：随机 256 位 DEK 加密字段；主密码经 Argon2id v19 派生 KEK 包装 DEK。默认建议从 `m=65536 KiB / t=3 / p=4 / 16-byte salt` 起步，M1 需实机测量并通过命名 profile 升级。
2. **每字段独立信封**：每次 AES-256-GCM 加密使用新的 12 字节 nonce 和 16 字节 tag；AAD 绑定 `vault_id / entry_id / field_id / crypto_version / record_version`。不保留共享 `entry_nonce`。记录更新递增版本并重加密该记录全部现存字段，在单事务提交。
3. **隐私优先数据格式**：title、URL、category、username、password、TOTP secret、notes 全加密；仅操作元数据明文。锁定时列表为空，搜索只在解锁后由后端分页解密执行，不创建明文 FTS。
4. **真实设备钥匙保护**：设备槽用独立 DKEK 包装 DEK。macOS DKEK 存 Keychain，采用 `WhenUnlockedThisDeviceOnly + biometryCurrentSet`；Windows DKEK 用当前用户 DPAPI 保护。认证成功 bool 永远不能从普通文件释放 DEK。
5. **Windows 保证边界**：推荐后端是“Hello 流程验证 + DPAPI 当前用户静态保护”，不冒称 Hello 私钥/TPM 在密码学上直接门控 DPAPI。若审查要求后者，需另做 KeyCredential 派生设计，不得静默提高宣传口径。
6. **锁定与清理**：后端 epoch 状态机防止迟到 KDF/认证结果；180 秒读取真实系统键鼠空闲，后台轮询不刷新。锁屏、睡眠、页面离开、关闭、停用立即锁；后端 `Zeroizing` 与前端释放/销毁分开描述，不承诺全进程绝对擦除。
7. **共享剪贴板**：要求 Codex 提供所有者 RAII 租约、操作 token 和平台修订号；30 秒只在 `operation_id` 与 `changeCount`/Windows 序号仍匹配时清除。翻译与密码租约可重叠，旧定时器不能清新复制，用户后续内容不能清除。
8. **恢复 fail closed**：结构/AEAD/版本错误不返回秘密、不自动覆盖原库；主密码为独立恢复路径。SQLite 原子事务、密文备份、损坏隔离、迁移失败回滚和设备槽重新注册分别验收。

## 需 Codex 审查和协调

### 密码学与格式决策

- 是否批准 Argon2id 默认 profile、逐字段表、完整 upsert 全记录重加密，以及全部用户内容加密的搜索取舍。
- 是否接受 Windows DPAPI + Hello 的明确安全边界；是否需要另立 Hello 密钥派生强化方案。
- 主密码变更后建议强制当前会话重锁一次；设备槽是否必须重新确认需冻结。
- 依赖级清理审查需覆盖 AES 扩展密钥、Argon2 工作内存和 FFI 缓冲，不能仅审 DEK 外层 `Zeroizing`。

### 公共接口/宿主决策

- 共享剪贴板 trait 最终归属、全局屏蔽期间的可用性取舍，以及 macOS 无跨进程原子 compare-and-clear、Windows 延迟呈现的 TOCTOU 边界。
- 现有迁移备份失败仅 warning：密码本破坏性/重加密迁移前必须改为或提供“备份失败即中止”路径。
- 当前共享连接 WAL + `synchronous=NORMAL` 可保证事务原子性，但最近提交的断电持久性不等于 FULL；是否调整由协调者决定，模块不私自切换全局 PRAGMA。
- 遗留顶层 `password_vault` 到 `modules.settings.password_vault` 的一次配置映射，以及 Host/目录/IPC/字典集成方式。

**本轮公共代码变更：无。**未修改 `EventBus`、模块 02、`Cargo.toml`、Host、OS HAL、配置或前端。

## 验证与未执行项

### 已执行

- `pwd`：返回指定专用 worktree。
- `git branch --show-current`：返回 `codex/feature-password-vault`。
- `git rev-parse HEAD`：开始前返回 `4689c49fc162eb4039b2a900d38d6f904c199a8d`，与基线一致。
- `git diff --cached --check`：设计提交前通过，无空白错误。
- 设计提交仅含 `docs/design/password-vault-m0.md`，512 行；提交正文记录 `Agent: WorkBuddy`、`Model: Kimi K3.1 Pro`。
- 任务派发文件 `docs/tasks/WORKBUDDY_VAULT_2026-10-25.md` 原本即为未跟踪文件，本轮未修改、未纳入提交。
- 已核对 RFC 9106、NIST SP 800-38D、RustCrypto、Apple Security/LocalAuthentication/AppKit、Microsoft DPAPI/Hello/输入与剪贴板、SQLite 官方资料，链接集中列于设计文档第 12 节。

### 未执行（符合 M0 门槛）

- 未编写或执行生产密码学、认证、钥匙串/DPAPI 逻辑，未挂载密码本模块。
- 未读取任何现有密码、浏览器密码、个人凭据或用户数据库；未使用真实秘密。
- 未触发生物识别/系统认证弹窗，未写入系统 Keychain、DPAPI 或真实剪贴板。
- 未运行 cargo/前端测试（本轮为文档唯一变更）；未执行原生集成或 GUI，所有测试矩阵均为计划，不能标为通过。
- macOS Touch ID、Windows Hello、真实 180 秒空闲、锁屏/睡眠、共享租约与 30 秒清理均未做真机验收。

## 下一阶段（仅在设计审查通过后）

1. 按审查意见冻结 v1 格式和公共契约，再实现 M1 主密码初始化、锁/解锁、CRUD/搜索、密码生成和 SQLite 迁移。
2. 完成 epoch 锁状态、真实系统空闲与生命周期清理、前端秘密短驻留；所有敏感 IPC 在锁定/停用状态拒绝。
3. 等 Codex 共享剪贴板租约落地后接入复制；此前不将复制能力标为可用。
4. 以虚构测试数据完成单元/集成测试后，再进入 M2 macOS Keychain 与 Windows DPAPI/Hello 原生适配及真机 GUI；未通过独立审查不投入真实秘密。

请 Codex 重点审查设计文档第 3–8 节和第 11 节。**当前停止在 M0，等待设计审查。**
