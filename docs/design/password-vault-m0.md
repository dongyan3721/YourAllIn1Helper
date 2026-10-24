# 功能 10：生物识别加密密码本 M0 设计

- 日期：2026-10-25
- 状态：**M0 待 Codex 设计审查；未实现、未挂载、未验收**
- 工作分支：`codex/feature-password-vault`
- 设计基线：`4689c49fc162eb4039b2a900d38d6f904c199a8d`
- 范围：密钥、密文与 SQLite 数据格式、IPC/状态契约、共享剪贴板需求、测试矩阵和 M1/M2 清单
- 非范围：生产密码学、认证/钥匙串写入、模块挂载、真实秘密或真实凭据测试

本设计只使用虚构标识和测试数据。审查通过前不得据此挂载空壳模块，也不得把现有 `password_vault` 配置项或目录槽位解释为功能已经可用。

## 1. 审查后应冻结的决策

1. 每个密码本初始化时生成一个随机的 256 位 DEK；字段加解密只使用该 DEK，主密码不直接充当字段密钥。
2. 主密码经 Argon2id v1.3（版本号 19）派生 256 位 KEK，KEK 以 AES-256-GCM 包装 DEK。默认参数从 RFC 9106 的第二推荐参数集起步：`m=65536 KiB`、`t=3`、`p=4`、16 字节盐、32 字节输出；必须在目标低配设备实测并记录结果，不能阻塞 UI 线程。
3. 选择**逐字段信封**，不采用说明书中的共享 `entry_nonce`。每次加密均独立生成 12 字节 nonce，使用 16 字节认证标签，并把密码本 ID、记录 ID、字段 ID、格式版本和记录版本编码进 AAD。
4. 标题、URL、分类、用户名、密码、TOTP 密钥、备注全部加密。明文只保留记录 ID、版本、时间戳、归档标记及密文元数据；搜索仅在解锁后于后端解密并匹配，不建立明文 FTS 索引。
5. 设备便捷解锁采用独立设备 KEK（DKEK）包装同一个 DEK：macOS 的 DKEK 放入带生物识别访问控制的 Keychain；Windows 的 DKEK 使用当前用户 DPAPI 保护。任何“认证成功 bool”都不能单独授权从一个普通可读文件取出 DEK/DKEK。
6. Windows 必须如实标注：DPAPI 提供当前用户/设备的静态秘密保护，`UserConsentVerifier` 提供每次解锁前的 Hello 用户验证，但二者组合**不等于**由 Hello 私钥在密码学上强制门控 DPAPI 解封；同用户恶意进程不在其防御范围内。M2 不以“TPM 绑定 DEK”宣传此方案。
7. 后端锁状态是唯一安全边界。180 秒指系统真实键鼠空闲，不是密码本 IPC 空闲；锁屏、睡眠、会话离开、页面离开、窗口关闭、模块停用和退出立即锁定。所有敏感 IPC 在锁定/停用状态拒绝。
8. 剪贴板由 Codex 的共享服务拥有。密码本要求所有者 RAII 屏蔽租约、带平台修订号的写入回执，以及 30 秒时“仍是本次复制才清除”的条件清理；不使用现有单个 `AtomicBool` 解决并发。
9. 数据损坏、AEAD 认证失败、设备槽失效均 fail closed。主密码是独立恢复路径；丢失主密码且所有设备槽不可用时，没有后门恢复。

## 2. 威胁模型与保证边界

### 2.1 设计要防御

- 攻击者复制应用 SQLite 数据库、配置文件和迁移备份，但没有主密码或原设备 OS 钥匙保护上下文。
- 离线修改密文字段、nonce、AAD 上下文、记录 ID 或字段归属，尝试记录/字段替换与篡改。
- 主密码错误、生物识别取消/失败、设备钥匙失效、崩溃和不完整迁移导致的不安全降级。
- 已锁定 WebView、后台 IPC、事件广播、日志及模块 02 历史记录意外获得秘密。
- 多次密码复制、翻译模拟复制及用户后续复制之间的延迟清理竞态。

### 2.2 明确不保证

- 不防御解锁后已控制进程的恶意代码、同用户恶意软件、管理员/root、内核、调试器、被替换的应用二进制、键盘记录器或恶意 WebView。
- 不防御屏幕截图、目标网站/应用获得用户主动粘贴的密码、操作系统或第三方剪贴板管理器、Universal Clipboard/云同步。模块 02 的历史屏蔽不代表其他软件不可读取系统剪贴板。
- `Zeroizing` 只清理其拥有的缓冲区；不能保证清除寄存器、编译器/移动/重分配产生的历史副本、交换分区、休眠文件、崩溃转储、FFI/WinRT/Keychain 内部副本、JavaScript 引擎副本或 GPU/桌面合成缓存。
- AES-GCM 认证单个字段及其上下文，但不防止攻击者删除记录、回滚整个数据库，或用另一份完整有效的旧记录连同其 ID/版本一起替换。没有外部可信状态就无法提供全库抗回滚。
- 进程崩溃或被强制终止后无法执行剪贴板延迟清理；重启时也不得在无法证明所有权的情况下清空用户剪贴板。
- 明文长度会由密文长度泄露；时间戳、记录数量及访问时序不属于加密隐藏范围。

## 3. 密钥体系与生命周期

### 3.1 密钥和随机源

| 名称 | 长度与来源 | 持久化 | 用途 |
| --- | --- | --- | --- |
| DEK | 32 字节，OS CSPRNG | 永不明文持久化；分别被 KEK/DKEK 包装 | 所有字段的 AES-256-GCM 数据密钥 |
| KEK | 32 字节，Argon2id(master password, salt, params) | 永不持久化 | 包装/解封主密码槽中的 DEK |
| DKEK | 每个设备槽独立 32 字节，OS CSPRNG | 只以 Keychain/DPAPI 保护形态存在 | 包装/解封对应设备槽中的 DEK |
| `kdf_salt` | 16 字节，OS CSPRNG | `vault_meta`，可公开 | Argon2id 盐；主密码变更时重新生成 |
| 字段 nonce | 12 字节，每次加密重新生成 | 对应字段行，可公开 | AES-GCM 唯一 IV |
| 包装 nonce | 12 字节，每次包装重新生成 | 对应包装槽，可公开 | KEK/DKEK 的 AES-GCM IV |
| `vault_id`、`entry_id`、`slot_id` | 随机 UUID，序列化时使用规范形式 | 可公开 | 域分隔、AAD 和设备槽标识 |

实现注入 `CryptoRng + RngCore`（生产使用 OS `OsRng`/`getrandom`，测试使用显式确定性适配器）。随机源失败必须中止初始化/写入，不能回退到时间戳、PID、计数器种子或固定测试 nonce。确定性随机适配器不得进入生产装配。

随机 96 位 nonce 的碰撞概率不为数学零。生产生成器不可恢复地失败时拒绝操作；测试必须注入重复 nonce 以验证防御性检查。实现可在当前 DEK 生命周期维护已用 nonce 集合并拒重，但内存集合不是跨重启证明，不能替代 CSPRNG。DEK 轮换时必须为所有字段生成全新 nonce。

### 3.2 Argon2id 参数与升级

默认 `kdf_profile=argon2id-v19-m65536-t3-p4`：

- `algorithm=argon2id`，Argon2 版本 `19`；输出 32 字节。
- `m_cost_kib=65536`、`t_cost=3`、`p_cost=4`、`salt_len=16`。
- 这是 RFC 9106 在较少内存场景的统一安全选项，不是所有设备永远足够的参数。M1 在 macOS/Windows 目标设备测量解锁耗时、峰值内存和 UI 响应；若调整，必须形成新的命名 profile，而非静默改变同一 profile 的含义。
- KDF 在受控 `spawn_blocking`/专用工作线程执行，限制并发为 1，避免连续解锁造成内存拒绝服务；UI 可取消等待，但底层 Argon2 调用完成后立即清理结果，不伪称可中途强制取消。
- 读取磁盘参数时先验证算法、版本、整数范围、盐长度和实现允许的上限，再分配内存。未知版本或超限参数拒绝，不直接按攻击者写入的超大 `m_cost` 分配。
- 参数编码进入主密码包装 AAD。修改参数、盐、包装 nonce 或密文最终都会使解封失败；攻击者仍可造成拒绝服务。
- 成功主密码解锁后，如果磁盘 profile 低于当前目标且已明确用户升级策略，则在同一写事务中用新盐/参数重新包装**同一个 DEK**；字段密文无需重写。升级失败保留旧槽，不能留下两套半提交元数据。
- 主密码变更同样生成新盐和新包装 nonce，只重包 DEK。DEK 轮换是另一操作：生成新 DEK，重新加密所有字段并重包所有主密码/设备槽，在一个 SQLite 事务中提交；任一失败全部回滚。

### 3.3 主密码初始化与恢复

初始化：校验主密码策略和确认值 → 生成 `vault_id`、DEK、盐 → 派生 KEK → 以独立 nonce 包装 DEK → 在一个事务中写入 `vault_meta` → 清理 KEK 和临时明文。初始化失败不留下“已初始化但不可解锁”的元数据。

主密码解锁：读取并验证元数据 → 派生 KEK → AES-GCM 解封 → 成功后才进入新的 `Unlocked(epoch)`。错误密码与主包装损坏对调用者统一为 `UnlockFailed`，日志不包含密码、KEK、DEK、盐推导结果或密文内容。

主密码恢复：设备认证不可用、设备钥匙被删除、生物识别集合变化或 DPAPI 无法解封时，始终可用正确主密码解封主槽。恢复后由用户显式重新注册设备槽。没有“跳过主密码”的调试路径，没有自动导入系统/浏览器密码，也没有主密码找回后门。

## 4. AEAD 信封与规范编码

### 4.1 为什么选逐字段而不是整条记录

| 方案 | 优点 | 代价 | 决策 |
| --- | --- | --- | --- |
| 逐字段信封 | 单个密码揭示无需解密整条记录；字段归属可由 AAD 精确绑定；空字段自然缺省 | nonce/tag/行数较多；记录版本变化需重包该记录所有现存字段 | **采用** |
| 整条记录一次加密 | 每记录仅一个 nonce/tag；记录内部一致性简单 | 揭示密码也必须解密全部字段；任何小编辑都解密/加密完整载荷；字段搜索仍需整记录解密 | 可作为未来新格式，不在 v1 混用 |

说明书的一个 `entry_nonce` 若同时用于 username/password/TOTP/notes，会在同一 DEK 下复用 GCM nonce，禁止照搬。v1 删除该列概念，不以“字段名拼接 nonce”“相同 nonce 不同 AAD”或“密文位置不同”作为替代；**每次加密调用都必须有新 nonce**。

### 4.2 字段信封

每个字段行保存：`crypto_version`、`record_version`、`nonce(12)`、`ciphertext`、`tag(16)`。认证标签单独存放，不依赖语言的拼接习惯；空秘密与“字段不存在”语义不同，空值也正常加密。明文统一为 UTF-8，单字段明文上限 64 KiB、单记录总明文上限 256 KiB，超出拒绝，避免异常数据耗尽内存。

AAD 是以下固定长度大端编码，禁止直接无分隔拼接可变字符串：

```text
"AIOVDF1"                    # 7 字节域分隔
|| crypto_version:u16be      # v1 = 1
|| vault_id:uuid_16_bytes
|| entry_id:uuid_16_bytes
|| field_id:u8               # 1=title, 2=url, 3=category, 4=username,
                             # 5=password, 6=totp_secret, 7=notes
|| record_version:u64be
```

- `record_version` 从 1 开始，每次内容更新递增并进入乐观并发条件。
- 更新一个记录时，后端解密其所有现存字段、应用完整 upsert、递增版本，并为**所有现存字段**重新生成 nonce 和密文，在同一事务写入。否则旧字段会因版本改变而无法认证。
- 解密时同时校验记录 ID、字段 ID、两个版本、nonce/tag 长度及字段白名单。把 password 密文移到 notes、移到另一记录或改变版本均认证失败。
- `created_at/updated_at/archived` 不进入字段 AAD；它们是可变操作元数据，不属于秘密。其恶意修改不能被字段 AEAD 检测，列入保证边界。

主密码包装 AAD：`"AIOVMW1" || crypto_version:u16be || vault_id:16 || canonical_kdf_profile_encoding`。设备包装 AAD：`"AIOVDW1" || crypto_version:u16be || vault_id:16 || slot_id:16 || backend_id:u8`。三种域分隔字符串不同，主密码、设备与字段包装不共享上下文。

### 4.3 解密失败处理

- 在标签验证成功前不释放任何明文到调用者；RustCrypto `aes-gcm` 的解密错误统一映射为内部认证失败。
- 主包装失败不进入解锁态；字段失败不返回该字段，并将本次会话转为锁定、记录不含秘密的错误码 `DataIntegrityError`。
- 不逐 bit/逐 tag byte 返回差异，不记录明文、完整密文或可离线加速分析的日志。
- 不用 SQLite 校验和代替 AEAD：前者检查结构/损坏，后者提供密码学认证，两者都需要。

## 5. 设备钥匙保护：macOS 与 Windows

### 5.1 统一设备槽模型

每个设备槽含随机 `slot_id`、后端标识、一个受 OS 保护的 DKEK，以及数据库中的 `DKEK` 包装 DEK 信封。设备解锁成功取得 DKEK 后仍需通过 AES-GCM 解封 DEK；认证结果本身永远不足以构造 DEK。

注册只能在已解锁会话中由用户显式确认：

1. 检查真实能力并生成新的 `slot_id`、DKEK、包装 nonce；不复用其他设备槽。
2. 先创建 OS 保护项；失败则数据库不变。
3. OS 项成功后在一个事务中写入设备包装和槽元数据；失败则尽力删除刚创建的 OS 项并保持主密码路径可用。
4. 若崩溃留下无数据库引用的 OS 项，它不能授权任何槽；下次注册按唯一 `slot_id` 检测并清理孤儿，不覆盖其他活动槽。

停用采用 fail-closed 顺序：先删除/失效 OS 项，再删除数据库槽；第二步失败会留下不可解封的槽并报错，不自动回滚成可用设备解锁。主密码恢复后可清理。设备槽元数据变化不修改主密码槽。

### 5.2 macOS Keychain（M2）

推荐后端 ID `macos_keychain_biometry_current_set_v1`：

- DKEK 作为 `kSecClassGenericPassword` 秘密项保存，`service` 使用应用稳定标识，`account` 使用随机 `slot_id`；不写入用户秘密以外的额外账户资料。
- 访问控制使用 `SecAccessControlCreateWithFlags`，可访问性为 `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`，约束为 `.biometryCurrentSet`。该项仅本设备、设备解锁时可用，并要求当前登记的生物识别集合；不同步/迁移到新设备。
- 每次解封使用新的 `LAContext`，Keychain 查询允许系统认证 UI；UI 能力探测与解封分开。不能把应用先调用 `evaluatePolicy` 得到 `true`，再读取一个无访问控制的 Keychain 项当成等价实现。
- 默认不把设备登录密码作为 Keychain 项的隐式回退；用户可始终选择应用主密码。若未来需要 `.userPresence`/设备密码路径，必须建立不同后端 ID 和独立审查，不能静默放宽 `.biometryCurrentSet`。
- 生物识别集合变化导致项不可用是预期 fail-closed 行为：不删除密码本，提示用主密码解锁并重新注册。
- `userCancel`、`authenticationFailed`、`biometryNotAvailable`、`biometryNotEnrolled`、`biometryLockout`、`interactionNotAllowed`、Keychain `errSecItemNotFound` 等映射为不同可操作错误，但均不解锁、不写日志秘密。
- Secure Enclave 可作为后续强化方案：官方支持 P-256 签名/ECDH，不可导入现有 DEK。M2 不宣称 DKEK 通用密码项本身位于 Secure Enclave；如改为 Secure Enclave ECDH 派生 DKEK，须新格式和独立密码学审查。

### 5.3 Windows DPAPI + Hello（M2）

推荐可交付基线后端 ID `windows_dpapi_current_user_hello_gate_v1`：

- 使用 `CryptProtectData` 保护 DKEK，**当前用户范围**，不设置 `CRYPTPROTECT_LOCAL_MACHINE`，不使用硬编码附加熵，不使用已弃用的 prompt flow；保存返回的完整不透明 blob。`CryptUnprotectData` 成功后立即把 DKEK 移入 `Zeroizing` 缓冲区并释放/清理 API 缓冲区。
- 每次设备解锁先通过桌面 interop 绑定 Dashboard HWND 调用 Hello `UserConsentVerifier`；只有 `Verified` 才继续 DPAPI 解封。`Canceled`、`DeviceBusy`、`DeviceNotPresent`、`DisabledByPolicy`、`NotConfiguredForUser`、`RetriesExhausted` 均不调用解封、不解锁。
- **真实边界**：DPAPI 确实使 DKEK 不是普通可读文件，并绑定当前 Windows 用户/设备；但它通常允许相同登录凭据在同一计算机解封，Hello `Verified` 是应用流程门控而非 DPAPI 的密码学访问控制。此方案可称“DPAPI 保护的 Windows Hello 便捷解锁”，不可称“每次 Hello 在 TPM 内直接释放 DEK”。
- Windows PIN/设备凭据变化、用户 Profile 漫游、系统重置或 DPAPI blob 损坏可能使槽不可用；行为是拒绝设备解锁并走主密码恢复。不得因 `Verified` 就忽略 DPAPI/AEAD 失败。
- `KeyCredentialManager`/`KeyCredential.RequestSignAsync` 可让 Hello 使用应用无法读取的 RSA 私钥，但 M0 不把“签名直接当 KEK”列为已批准设计：签名算法的确定性、域分隔及把签名当秘密输入需要独立密码学论证。`RequestDeriveSharedSecretAsync` 的公开参考目前也不足以冻结跨版本格式。任何 Hello 密钥派生强化另开 M2 子设计和真机审查，不影响上述 DPAPI 边界的如实表述。

## 6. 锁状态、内存和 IPC

### 6.1 状态机与竞态

状态：`Uninitialized → Locked ⇄ Unlocking → Unlocked(epoch) → Locking → Locked`；模块 `Disabled` 是独立路由边界。每次成功解锁递增 `epoch`，每次锁定先使当前 epoch 失效，再清理密钥。

- 一个互斥状态机串行化解锁/锁定/停用转换；同一时刻最多一个主密码 KDF 和一个设备解封操作。
- 异步操作入口记录 epoch，在 KDF/原生认证/数据库等待之后、敏感返回或写事务提交之前再次检查。锁定时递增 epoch 并取消等待者；迟到的认证成功、KDF 结果、数据库结果和 IPC 响应全部丢弃并清理。
- 锁屏、睡眠、会话切换、窗口关闭、路由离开、模块停用、退出均调用同一幂等 `lock(reason)`；后端先于事件发布清理 DEK/DKEK/KEK 和待处理明文。
- `on_shutdown` 停止定时器/原生观察者，拒绝新操作，等待已进入操作按生命周期契约退出，再最终清理。不能依赖前端一定来得及发送 lock。
- 原生空闲查询失败采取 fail-closed 锁定并报告能力错误，不能把 `Unsupported` 当成“始终活跃”。M1 在真实原生空闲/锁屏适配器未验收的平台不宣称完整自动锁已通过。

### 6.2 180 秒真实用户空闲

- 使用系统键鼠空闲源：macOS `CGEventSourceSecondsSinceLastEventType(..., kCGAnyInputEventType)`；Windows 当前会话 `GetLastInputInfo`。后端比较 `idle_seconds >= 180`，使用单调计时辅助，不依赖墙上时钟。
- 每秒查询只读空闲值，**绝不刷新它**。密码本 IPC、前端轮询、 reveal 倒计时和后台任务均不重置系统空闲时间。
- 锁屏/睡眠由原生事件立即触发：macOS NSWorkspace 屏幕睡眠与会话离开通知；Windows `WM_WTSSESSION_CHANGE(WTS_SESSION_LOCK)`、会话断开和电源睡眠通知。恢复时首先再次检查锁状态和空闲值。
- Windows `GetLastInputInfo` 仅覆盖调用会话且 tick 不保证单调，实现需按其文档处理；不能把它包装成跨所有会话的全局输入监控。
- 默认值固定 180 秒；M1 不开放更小安全语义不明的自定义档位。若以后允许配置，持久化值只能是偏好，锁定事件和当前剩余时间不得被客户端用来推迟后端锁定。

### 6.3 后端与前端清理

后端：DEK/KEK/DKEK 使用 `Zeroizing<[u8; 32]>`；明文优先预分配 `Zeroizing<Vec<u8>>`，避免 `String`/`Vec` 扩容和 `Clone`；秘密类型不实现 `Debug`、`Serialize` 或日志格式化。锁时丢弃所有临时缓冲区、编辑快照、搜索结果、密码生成缓存和异步结果。还需审查 `Aes256Gcm` 对象内部扩展密钥是否/何时清零，不能只包装 DEK 字节就宣称所有密钥材料已擦除。

前端：秘密不进入 Vue 持久 store、路由 query、localStorage、日志、错误上报或事件广播；揭示值只在局部组件按需短驻留，最长 15 秒后清除。锁定、路由离开、`pagehide`、窗口隐藏/关闭和停用立即清空密码输入、揭示值、编辑表单、搜索结果、TOTP 和剪切板相关 UI 状态，再导航到锁屏。窗口销毁由宿主后端同时强制 lock。

JavaScript 字符串不可可靠原地覆写，Tauri IPC 的序列化/WebView/FFI 缓冲也不能由 `Zeroizing` 覆盖。因此“前端清理”指释放引用、覆盖可见状态并销毁 WebView，不宣称物理擦除所有 JS/IPC 副本。

### 6.4 IPC 契约（设计名）

| IPC | 允许状态 | 最小输入/返回与要求 |
| --- | --- | --- |
| `vault_get_status` | 非停用 | 仅 `initialized/locked/unlocking`、设备能力与可用后端；不返回条目或秘密 |
| `vault_initialize` | 未初始化 | 输入主密码及确认；只返回新状态，不返回 DEK/元数据秘密 |
| `vault_unlock_master_password` | 已锁定 | 输入主密码；统一 `UnlockFailed`；返回新 epoch 状态 |
| `vault_unlock_biometric` | 已锁定且槽可用 | 不输入秘密；取消/不可用/失效均不解锁 |
| `vault_lock` | 任意非停用 | 幂等；无敏感返回 |
| `vault_list_entries(search, cursor, limit)` | 已解锁 | 后端解密搜索；分页仅返回展示所需 ID/title/username/URL/category/时间，不含 password/TOTP/notes |
| `vault_get_entry(id)` | 已解锁 | 仅显式编辑时返回完整表单；前端按敏感短驻留处理 |
| `vault_reveal_password(id)` | 已解锁 | 只返回该密码和短寿命提示；不广播；到期/锁定清除 |
| `vault_copy_password(id)` | 已解锁 | 秘密只进入共享剪贴板服务；返回 `{operation_id, clear_after_secs}`，不返回密码 |
| `vault_upsert_entry(input, expected_version)` | 已解锁 | 完整记录更新、乐观并发、全字段新 nonce；返回 ID/新版本 |
| `vault_delete_entry(id, expected_version)` | 已解锁 | 事务删除；返回非敏感结果 |
| `vault_change_master_password(old, new)` | 已解锁 | 重包同一 DEK；成功后可保持当前 epoch 或按审查决定强制重锁 |
| `vault_set_biometric_enabled(enabled)` | 已解锁，M2 | 管理真实 OS 槽，不能只翻转配置 bool |
| `vault_generate_password(policy)` | 模块启用 | 使用 CSPRNG；结果只返回调用者且不记日志；不自动保存 |

所有业务 IPC 由宿主 `with_enabled("password_vault", ...)` 进入，并在模块内再次验证 epoch。锁定态对列表、搜索、读取、揭示、复制、增删改和设备注册统一返回 `Locked`；不允许通过另一 IPC 或旧响应绕过。

事件仅 `vault://locked {epoch, reason}` 和可选 `vault://changed {epoch}`，不包含秘密、条目内容或错误详情。`reason` 使用有限枚举：`manual/idle/screen_lock/sleep/page_close/module_disabled/integrity_error/shutdown`。

## 7. 共享剪贴板租约与条件清理（提交 Codex 的契约需求）

本设计不修改 `EventBus`、模块 02 或公共 Rust 文件。以下是密码本对 Codex 共享服务的最低接口需求，具体类型归属由 Codex 决定：

```rust
// 契约示意，不是本轮生产代码。
trait ClipboardProtection: Send + Sync {
    fn acquire_suppression(&self, owner: ClipboardOwner) -> Result<SuppressionLease>;
    fn write_secret_text(
        &self,
        lease: &SuppressionLease,
        value: Zeroizing<String>,
    ) -> Result<ClipboardWriteToken>;
    fn clear_if_current(&self, token: &ClipboardWriteToken) -> Result<ClearOutcome>;
}

struct ClipboardWriteToken {
    operation_id: Uuid,       // 本次密码复制，旧定时器不能冒充新操作
    owner_id: Uuid,           // 对应租约所有者
    platform_revision: u64,   // 写入成功后的系统剪贴板修订号
}

enum ClearOutcome { Cleared, AlreadyChanged, LeaseExpired, Unsupported }
```

契约要求：

1. `SuppressionLease` 是 RAII、不可伪造的所有者租约；内部按 owner/token 引用计数。翻译与密码本可并存，任一释放只减少自己的引用，最后一个释放才恢复模块 02 捕获。异常、取消、停用和 Drop 均释放，不能广播秘密。
2. 获取租约成功后才解密并写系统剪贴板；租约覆盖写入到条件清理完成的整个秘密暴露期。若所有权提前丢失，共享服务可提前释放；最迟在 30 秒清理后释放。
3. 写入返回系统修订号：macOS 使用 `NSPasteboard.changeCount`，Windows 使用 `GetClipboardSequenceNumber`。本地 `operation_id` 防止同一模块旧定时器清除新复制。
4. 30 秒到期由共享服务串行执行 `clear_if_current`：仅当租约/操作有效且当前平台修订号等于写入回执时才清空；不相等返回 `AlreadyChanged`，**绝不恢复旧剪贴板，也绝不清除用户后续内容**。
5. Windows 在持有剪贴板打开句柄的清理区间内复查修订号再 `EmptyClipboard`；macOS 在 AppKit 要求的线程比较 `changeCount` 后调用 `clearContents`。官方没有提供跨进程原子 compare-and-clear，因此 macOS 检查和清空之间及 Windows 延迟呈现场景仍有极小的系统级 TOCTOU 边界，必须记录并做原生压力测试，不能宣称绝对无竞态。
6. 手动锁定、窗口关闭、模块停用/退出时，可凭仍有效的 token 提前执行同一条件清理；进程崩溃无法保证。到期任务不得因为另一次复制而被盲目取消，必须由 token 判定。
7. 在租约活动期模块 02 历史捕获保持屏蔽，避免密码进入本应用历史；这也意味着并发期间其他普通复制可能不被模块 02 记录，属于共享全局屏蔽的可用性取舍。Codex 如改为按修订号精确过滤，需保持与翻译模块兼容并另测。
8. 共享服务未交付前，`vault_copy_password` 不得作为正式可用能力；不能用 `set_clipboard_suppression(true/false)` 或当前 `AtomicBool` 临时替代。

## 8. SQLite 数据格式、迁移与恢复

### 8.1 v1 表（逻辑设计）

```sql
-- 由 ModuleMigration("password_vault", 1, ...) 所有；不是本轮执行 SQL。
vault_meta(
  id INTEGER PRIMARY KEY CHECK(id = 1),
  vault_id TEXT NOT NULL UNIQUE,
  crypto_version INTEGER NOT NULL CHECK(crypto_version = 1),
  kdf_profile TEXT NOT NULL,
  kdf_m_cost_kib INTEGER NOT NULL,
  kdf_t_cost INTEGER NOT NULL,
  kdf_p_cost INTEGER NOT NULL,
  kdf_salt BLOB NOT NULL CHECK(length(kdf_salt) = 16),
  master_wrap_nonce BLOB NOT NULL CHECK(length(master_wrap_nonce) = 12),
  master_wrapped_dek BLOB NOT NULL CHECK(length(master_wrapped_dek) = 48),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

password_vault_entries(
  id TEXT PRIMARY KEY,
  vault_id TEXT NOT NULL,
  record_version INTEGER NOT NULL CHECK(record_version >= 1),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  archived INTEGER NOT NULL DEFAULT 0 CHECK(archived IN (0, 1))
);

password_vault_fields(
  entry_id TEXT NOT NULL REFERENCES password_vault_entries(id) ON DELETE CASCADE,
  field_id INTEGER NOT NULL CHECK(field_id BETWEEN 1 AND 7),
  crypto_version INTEGER NOT NULL CHECK(crypto_version = 1),
  record_version INTEGER NOT NULL CHECK(record_version >= 1),
  nonce BLOB NOT NULL CHECK(length(nonce) = 12),
  ciphertext BLOB NOT NULL,
  tag BLOB NOT NULL CHECK(length(tag) = 16),
  PRIMARY KEY(entry_id, field_id),
  UNIQUE(entry_id, nonce)
);

vault_device_slots(
  slot_id TEXT PRIMARY KEY,
  vault_id TEXT NOT NULL,
  backend TEXT NOT NULL,
  crypto_version INTEGER NOT NULL,
  device_wrap_nonce BLOB NOT NULL CHECK(length(device_wrap_nonce) = 12),
  device_wrapped_dek BLOB NOT NULL CHECK(length(device_wrapped_dek) = 48),
  os_protected_dkek BLOB,          # Windows DPAPI blob；macOS DKEK 在 Keychain
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
```

`UNIQUE(entry_id, nonce)` 只捕获单记录明显错误，不是全 DEK nonce 唯一性的充分保证；不得把它当作安全证明。UUID 在写入前解析为规范 16 字节用于 AAD，数据库 TEXT 仅使用小写规范形式。外键每连接开启；记录和字段的 `vault_id/record_version` 一致由事务内业务校验，必要时后续加复合外键。

### 8.2 明文与搜索隐私取舍

- 可明文：`vault_id/entry_id/slot_id`、KDF/AEAD 参数、盐、nonce、密文/tag、设备后端名、记录版本、创建/修改时间、归档位。
- 必须解密：title、URL、category、username、password、TOTP secret、notes，以及未来任何用户输入的标签/自定义字段。
- 锁定态列表为空，不能利用明文 title/URL 浏览；解锁后后端按页解密搜索，限制 `limit`（默认 50、最大 100），排序只使用 ID/时间等操作元数据。
- 不为搜索建立明文 trigram/FTS 表，不把 title/domain 做“可忽略隐私”的例外。代价是大数据量搜索较慢且密文长度仍可见；后续如需索引必须另审确定性加密泄露，不在 v1 偷加。

### 8.3 原子更新与六模块隔离

- 复用 `_schema_migrations(module_id, version)`，模块 ID 固定 `password_vault`。v1 仅新增上述表和索引，不修改/重命名/删除现有六模块表、配置或迁移记录。
- 初始化、记录 upsert/delete、主密码重包、DEK 轮换、设备槽数据库更新分别在 SQLite 事务中完成；记录版本条件更新影响行数不为 1 时返回 `VersionConflict` 并回滚。
- 迁移只追加有序版本并验证已应用 SQL checksum；发现同版本 checksum 不同、未知未来版本或不完整格式时停止密码本初始化，不尝试“自动修复”。
- 现有宿主在迁移前会尝试 SQLite Online Backup，但备份失败只记录 warning 并继续。对含密码本的**破坏性/密码学迁移**这不满足恢复门槛：M1 新增空表可评估风险；任何后续重加密迁移前必须由 Codex/宿主提供“备份失败即中止”的策略，不能静默继承 warning。
- 当前宿主为 WAL + `synchronous=NORMAL` + `foreign_keys=ON`。事务提供原子性，但 SQLite 官方说明 WAL/NORMAL 在断电后可能丢失最近已提交事务；不得宣称每次写入都具备 FULL 的断电持久性。是否对关键事务/共享连接采用 FULL 是协调者决策，不能由模块私自反复修改全局 PRAGMA。
- 停用模块不删除表；已挂载模块迁移仍在启动执行。未挂载前不创建密码本表，也不改变目录 `available`。

### 8.4 损坏、备份与恢复

1. 启动执行 schema/长度/版本检查；迁移前和显式诊断时运行 `PRAGMA integrity_check` 与 `PRAGMA foreign_key_check`。`integrity_check` 不覆盖外键，也不证明密文真实。
2. 任一结构错误、认证失败或不支持的密文版本：不返回秘密、不覆盖原库、不自动删除设备槽；锁定并给出不含秘密的恢复错误。
3. 备份使用 SQLite Online Backup 生成一致性密文快照，包含主密码包装；设备槽依赖本机 OS 状态，跨机不可假设可恢复。备份文件同等敏感，不放入仓库、日志或测试附件。
4. 恢复到隔离的新数据库并验证后才允许用户显式替换；先保留原库及关联 `-wal/-shm` 的一致性快照，不能在原库上边校验边破坏性修复。恢复后默认重新注册设备槽。
5. 主密码可恢复密文备份；Keychain/DPAPI 槽只在原用户/设备/访问控制仍有效时可用。主密码丢失且无可用设备槽时明确不可恢复。
6. `secure_delete=ON` 可减少普通 SQLite 页中已删除内容残留，但不自动覆盖所有 WAL、备份、虚拟表或文件系统副本。v1 不启用明文 FTS；是否开启 `secure_delete` 及何时 checkpoint/VACUUM 需集成决策，不能宣称物理安全擦除。

## 9. 模块接口、配置与实施清单

### 9.1 依赖注入（建议）

```rust
// 接口草案，具体可见性/异步运行时由 M1 审查后确定。
trait VaultKeyStore {
    fn backend_id(&self) -> &'static str;
    fn capability(&self) -> Result<DeviceCapability>;
    fn protect(&self, slot: SlotId, dkek: Zeroizing<[u8; 32]>) -> Result<ProtectedDkek>;
    fn unprotect(&self, slot: SlotId, protected: &ProtectedDkek) -> Result<Zeroizing<[u8; 32]>>;
    fn revoke(&self, slot: SlotId) -> Result<()>;
}

trait VaultClock {
    fn monotonic_now(&self) -> Instant;
    fn system_idle_seconds(&self) -> Result<u64>;
}

trait VaultRepository { /* 初始化、事务 CRUD、版本检查、迁移读取 */ }
trait VaultCrypto { /* 纯函数信封、KDF profile、显式测试 RNG 注入 */ }
```

`BiometricAuthenticator.verify_identity() -> bool/BiometricResult` 只能作为能力/用户验证接口，**不是** `VaultKeyStore.unprotect`。M2 将原生访问控制封装在独立平台 `VaultKeyStore` 适配文件中；业务 crate 不直接导入 AppKit/Win32。测试替身只能显式注入，生产没有 Mock 回退。

### 9.2 配置约定

新配置归属 `modules.settings.password_vault`，由模块自己的 `config.rs` 定义：

```json
{
  "auto_lock_seconds": 180,
  "clipboard_clear_seconds": 30,
  "biometric_preferred": true
}
```

- 仅保存非秘密偏好；主密码、DEK/KEK/DKEK、TOTP、设备 blob 不得进入 `config.json`。
- `biometric_preferred` 只控制 UI 选择，不能替代真实设备槽，也不能使不可用后端变成可用。
- 现有顶层 `password_vault` 是遗留 typed 配置。集成时由 Codex 决定一次映射到模块设置并保留未知字段；不能让两处配置产生不同安全语义。
- 时间阈值 v1 固定要求 180/30；配置读取错误使用安全默认并报告，不接受 0、负数、溢出或无限延迟。

### 9.3 M1：主密码闭环（审查通过后）

- [ ] 新建真实 `mod-password-vault` 及纯格式/密钥接口，但只有在实现完整最小闭环后才挂载 `available`；不先挂空壳。
- [ ] 加入经审查的依赖：`argon2`、`aes-gcm`、`rand_core/getrandom`、`zeroize`，复用 workspace 的 `serde/rusqlite/uuid/tokio`；锁定确切版本并审查 feature/MSRV/供应链，不引入另一套 AEAD。
- [ ] 实现 v1 迁移、初始化、主密码锁/解锁、字段 CRUD、解锁搜索、密码生成、版本冲突和事务恢复。
- [ ] 实现 epoch 状态机、后端清理、前端短驻留清理及原生 180 秒空闲、锁屏/睡眠/关闭/停用路径。
- [ ] 接入 Codex 共享剪贴板服务；未完成前复制能力保持不可交付。
- [ ] 仅使用内存/临时目录中的虚构数据完成单元与集成测试；不调用真实认证、不写系统 Keychain/DPAPI。
- [ ] 公共清单/锁文件、目录、Host 装配、IPC 路由、三语字典、共享 OS HAL/配置迁移由协调者统一处理或审查增量；本功能不得修改 EventBus/模块 02。

### 9.4 M2：真实设备解锁与防泄漏

- [ ] macOS 独立适配：Keychain ACL + `LAContext` + DKEK 设备槽；取消/不可用/集合变化/撤销/重装/主密码恢复。
- [ ] Windows 独立适配：桌面 HWND interop 的 Hello + 当前用户 DPAPI DKEK；所有结果分支、Profile/设备变化、取消/撤销/恢复。
- [ ] 原生认证异步不阻塞 AppKit/Tauri 主事件循环；迟到结果带 epoch，取消和停用清理。
- [ ] 在专用虚构 QA 数据、专用 Keychain service/Windows 测试用户上下文做显式原生测试；不得读取既有钥匙串项或触发非测试账户认证。
- [ ] 完成篡改、nonce 注入、锁竞态、重启完整性、复制并发、崩溃恢复测试，再分别执行 macOS/Windows 真机 GUI。
- [ ] 未实测的平台、未验证的硬件/Windows 版本在交接逐项列出；未通过独立审查前不开放真实秘密使用。

## 10. 测试矩阵与证据分层

以下全部是**计划，不是已执行结果**。单元测试可用显式虚构适配器；原生集成和 GUI 证据不得互相替代。当前 M0 未执行任何认证、Keychain/DPAPI 写入或真实秘密测试。

### 10.1 单元与普通集成测试（无桌面权限）

| ID | 场景/注入 | 必须断言 | 证据层 |
| --- | --- | --- | --- |
| U-01 | 初始化虚构密码本 | DEK 随机且不明文落盘；盐/包装 nonce 长度正确；重启可解锁 | 单元+SQLite |
| U-02 | Argon2id 官方/项目固定向量与 profile 编解码 | 输出/参数精确匹配；未知版本、盐长度、超界 m/t/p 拒绝 | 单元 |
| U-03 | 错误主密码 | 不解锁、不返回部分条目；临时 KEK 被清理；错误不泄露区分信息 | 单元 |
| U-04 | 同一虚构字段连续加密两次 | key 相同但 nonce/密文不同；两者均可解密 | 单元 |
| U-05 | 同记录 username/password/TOTP/notes | 不存在共享 `entry_nonce`；每次调用 nonce 独立 | 单元 |
| U-06 | 测试 RNG 返回重复 nonce | 操作拒绝或触发明确防御，不输出第二份同 key/nonce 密文 | 单元 |
| U-07 | 随机源失败 | 初始化/写入/轮换失败且不产生半状态，无弱随机回退 | 单元 |
| U-08 | 修改 ciphertext/tag/nonce 任意字节 | AEAD 失败、零明文返回、会话锁定 | 单元 |
| U-09 | password 密文放到 notes/另一 entry/另一 vault | AAD 不匹配并拒绝 | 单元 |
| U-10 | 修改 record_version/crypto_version/field_id | 拒绝；不静默按新上下文解释 | 单元 |
| U-11 | 完整有效旧记录连同 ID 替换 | 测试记录“可接受但不防伪”的边界，不误报为已防御 | 单元 |
| U-12 | 主包装 KDF 参数/包装 AAD 篡改 | 有界参数检查后拒绝或认证失败，无大额不受控分配 | 单元 |
| U-13 | 主密码变更/KDF 升级 | DEK 不变，旧包装失效，新包装可解锁；字段无需重写 | SQLite |
| U-14 | DEK 轮换中途第 N 个字段失败 | 全事务回滚；旧 DEK/字段仍一致；无混合版本 | SQLite 故障注入 |
| U-15 | upsert 只改一个字段 | 所有现存字段版本一致且均为新 nonce；旧版本不能解密 | SQLite |
| U-16 | 两个相同 expected_version 更新 | 仅一个成功；另一个 `VersionConflict` | SQLite 并发 |
| U-17 | 初始化/重包提交失败或进程崩溃点注入 | 全有或全无；重启后旧/新状态之一完整可用 | SQLite |
| U-18 | 六模块夹具 + v1 密码本迁移 | 旧表/行/配置/迁移 checksum 不变，新增仅 password_vault | SQLite |
| U-19 | 已应用同版本 SQL 被改动、未来版本、缺字段 | fail closed，不自动修补 | SQLite |
| U-20 | 损坏页、外键错误、非法 blob 长度 | 结构检查与 AEAD 分层发现；不覆盖原库 | SQLite |
| U-21 | 备份失败 | 新增空表行为符合审查；破坏性迁移必须中止 | SQLite 故障注入 |
| U-22 | 密文备份恢复 | 主密码可解锁；设备槽不被当成可跨机恢复 | SQLite |
| U-23 | 锁定后逐项调用所有敏感 IPC | 全部 `Locked`，无条目/秘密/旧响应 | IPC |
| U-24 | 模块停用与敏感 IPC 竞态 | 新调用拒绝，已进入调用清理；不返回秘密 | 生命周期 |
| U-25 | KDF/认证途中 lock，随后迟到成功 | epoch 失效，结果清零，不能进入解锁态 | 并发 |
| U-26 | reveal/copy/upsert 与 lock 同时发生 | 返回/提交前复查 epoch；无锁后敏感响应 | 并发 |
| U-27 | 注入系统空闲 179→180，同时持续 IPC/轮询 | 179 不锁，达到 180 锁；IPC 不刷新真实空闲 | 单元 |
| U-28 | 锁屏/睡眠/离开/关闭/停用重复事件 | 幂等立即锁；后端先清理再发事件 | 生命周期 |
| U-29 | 空闲查询 `Unsupported`/错误 | fail closed 且显示能力缺口，不假装活跃 | 单元 |
| U-30 | `Zeroizing` 自有测试缓冲区 Drop | 拥有缓冲区清零；另列 FFI/JS/扩展密钥不可证明项 | 单元+代码审查 |
| U-31 | 搜索 title/username/URL/notes | 仅解锁后匹配；无明文 FTS/索引；分页上限生效 | SQLite |
| U-32 | 日志/事件捕获 | 不含主密码、秘密、密钥、完整敏感 IPC 参数 | 集成 |

### 10.2 共享剪贴板契约测试（由 Codex 主责，密码本提供用例）

| ID | 场景 | 必须断言 |
| --- | --- | --- |
| C-01 | 翻译租约 A 与密码租约 B 重叠，A 先释放 | B 期间仍屏蔽；B 释放后才恢复 |
| C-02 | 同一所有者嵌套/异常/Drop/停用 | 引用计数正确，无提前放开或永久屏蔽 |
| C-03 | 密码复制后 30 秒未变 | revision 匹配才清空，租约随后释放 |
| C-04 | 第 10 秒用户复制其他内容 | 到期 `AlreadyChanged`，用户新内容保留 |
| C-05 | 第 10 秒第二次密码复制，第一次定时器到期 | 旧 token 不清新内容；仅新 token 到期可清 |
| C-06 | macOS `changeCount` / Windows 序号变化 | 使用真实平台修订号而非文本比较或 AtomicBool |
| C-07 | 清理与第三方写入压测 | 记录平台 TOCTOU 结果；发现缺陷不得宣称竞态已消除 |
| C-08 | lock/关闭/停用/进程崩溃 | 前三种尽力立即条件清理；崩溃残留如实记录，不重启乱清 |
| C-09 | 模块 02 历史与第三方管理器 | 本应用历史无秘密；不声称第三方不可见 |
| C-10 | 未获取租约/写入失败/修订号不可用 | 不执行秘密复制正式流程，返回错误并清理明文 |

### 10.3 原生集成（M2，显式运行、虚构 QA 数据）

| 平台 | 必测证据 |
| --- | --- |
| macOS | 真实 ACL 项注册/读取/撤销；Touch ID 成功、取消、失败、锁定、未登记；生物识别集合变化后失效；主密码恢复；无 ACL 直接读取不能替代；休眠/会话离开；`changeCount` 清理。记录 macOS、硬件、签名/entitlements、专用 service 名称和测试日志，不附秘密。 |
| Windows | Hello 成功及每种可用性/取消结果；DPAPI 当前用户 blob 解封、损坏、其他用户/设备不可用；Profile/PIN 变化后恢复；锁屏/睡眠/会话断开；剪贴板序号及延迟呈现。记录 Windows build、Hello 类型、TPM 有无和测试用户边界，不附秘密。 |

原生测试只能由后续明确授权触发系统 UI；当前 M0 不执行。Secure Enclave/TPM 若未实际使用，不得写入“已通过”。

### 10.4 GUI 验收（M1/M2 分阶段）

- 初始化、错误主密码、锁/解锁、CRUD/搜索/生成器、重启持久化；所有值均虚构。
- reveal 15 秒清除；真实键鼠空闲 180 秒（持续后台 IPC 不延缓）；锁屏、睡眠、路由离开、窗口关闭、总开关停用立即锁并清屏。
- 生物识别成功/取消/不可用/设备变化与主密码恢复；未测平台单独列出。
- 两次秘密复制、翻译重叠、用户后续复制、30 秒条件清理及模块 02 历史截图。
- 损坏/篡改恢复不显示秘密、不覆盖原库；升级后旧六模块数据仍在。
- 每项附实际构建 SHA-256、日期、OS/硬件、截图或录像及测试数据隔离目录；IPC/单测不能代替真实点击和实体键鼠证据。

## 11. 需 Codex/协调者审查决策

1. 是否批准 v1 默认 Argon2id profile、逐字段表、完整 upsert 时全记录重加密及全部用户内容加密的搜索取舍。
2. 是否接受 Windows “DPAPI 当前用户保护 + Hello 流程门控”的明确安全级别；若要求密码学上强制 Hello 才能取钥，则 Windows M2 必须另做 Hello KeyCredential 派生设计，不能沿用本后端 ID。
3. 共享剪贴板 trait 的最终归属、全局屏蔽可用性取舍、平台 compare-and-clear 限制是否可接受；未解决前复制不可交付。
4. 现有迁移备份失败仅 warning 的策略需要为密码本破坏性迁移增加强制中止；共享连接 `synchronous=NORMAL` 是否调整由协调者评估。
5. 遗留顶层 `password_vault` 配置到 `modules.settings.password_vault` 的一次迁移，以及 Host/目录/IPC/字典的公共变更清单。
6. `Aes256Gcm` 扩展密钥、Argon2 工作内存、原生 FFI 缓冲、JS/IPC 副本的依赖级清理审查；`Zeroizing` 不得被写成全进程绝对擦除保证。
7. 主密码变更后是否强制全部设备槽重新确认、是否保持当前解锁 epoch；默认建议主密码重包成功保持设备槽但强制当前会话重锁一次。

## 12. 官方与实现依据

以下链接于 2026-10-25 核对；实现时仍需锁定实际 crate/API 版本并复核桌面目标支持。

### 密码学与随机/清理

1. RFC Editor, [RFC 9106: Argon2 Memory-Hard Function](https://www.rfc-editor.org/rfc/rfc9106.html)，§4：Argon2id 推荐参数、128-bit salt、256-bit tag 及内存清理建议。
2. NIST, [SP 800-38D: Galois/Counter Mode (GCM) and GMAC](https://doi.org/10.6028/NIST.SP.800-38D)，§8/§8.2：同一 key 下 IV 唯一性与 96-bit IV 构造；AAD 只认证不加密。
3. RustCrypto, [`aes-gcm`](https://docs.rs/aes-gcm/latest/aes_gcm/)：推荐 96-bit nonce、128-bit tag、每次消息唯一 nonce、AAD/解密错误接口。
4. RustCrypto, [`zeroize`](https://docs.rs/zeroize/latest/zeroize/) 与 [`Zeroizing`](https://docs.rs/zeroize/latest/zeroize/struct.Zeroizing.html)：编译器不可优化掉的清理及其 Vec/String 重分配、寄存器等保证边界。
5. RustCrypto `rand_core`, [`OsRng`](https://docs.rs/rand_core/latest/rand_core/rngs/struct.OsRng.html)：由 OS 提供的 CSPRNG 接口。

### Apple

6. Apple, [`SecAccessControlCreateFlags`](https://developer.apple.com/documentation/Security/SecAccessControlCreateFlags)：`.biometryCurrentSet`、`.userPresence` 等 Keychain 访问约束。
7. Apple, [`kSecAttrAccessibleWhenUnlockedThisDeviceOnly`](https://developer.apple.com/documentation/security/ksecattraccessiblewhenunlockedthisdeviceonly)：仅解锁时、仅本设备、不迁移。
8. Apple, [Keychain data protection](https://support.apple.com/guide/security/secb0694df1a)：钥匙串可访问性类别与设备密钥包边界。
9. Apple, [Logging a User into Your App with Face ID or Touch ID](https://developer.apple.com/documentation/localauthentication/logging-a-user-into-your-app-with-face-id-or-touch-id) 与 [`LAPolicy.deviceOwnerAuthenticationWithBiometrics`](https://developer.apple.com/documentation/LocalAuthentication/LAPolicy/deviceOwnerAuthenticationWithBiometrics)：能力检查、取消/失败和生物识别专用策略。
10. Apple, [Protecting keys with the Secure Enclave](https://developer.apple.com/documentation/security/protecting-keys-with-the-secure-enclave)：P-256、不可导入现有密钥、不可导出私钥及硬件限制。
11. Apple, [`NSPasteboard.changeCount`](https://developer.apple.com/documentation/appkit/nspasteboard/1533544-changecount)：记录并比较所有权变化。
12. Apple, [`NSWorkspace.screensDidSleepNotification`](https://developer.apple.com/documentation/AppKit/NSWorkspace/screensDidSleepNotification)：工作区睡眠通知；Quartz `CGEventSourceSecondsSinceLastEventType` 用于输入空闲。

### Microsoft

13. Microsoft, [`CryptProtectData`](https://learn.microsoft.com/en-us/windows/win32/api/dpapi/nf-dpapi-cryptprotectdata)：当前用户/设备绑定、MAC、`CRYPTPROTECT_LOCAL_MACHINE` 风险和 UI flag。
14. Microsoft, [`UserConsentVerifier`](https://learn.microsoft.com/en-us/uwp/api/windows.security.credentials.ui.userconsentverifier) 与 [Fingerprint biometrics](https://learn.microsoft.com/en-us/windows/apps/develop/security/fingerprint-biometrics)：Hello 可用性、Verified/Canceled 等结果；桌面应用使用对应 HWND interop。
15. Microsoft, [Windows Hello / KeyCredential](https://learn.microsoft.com/en-us/windows/apps/develop/security/windows-hello)：Hello 私钥不可由应用直接读取，`RequestSignAsync` 的用户验证和 TPM/软件回退边界。
16. Microsoft, [`GetLastInputInfo`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getlastinputinfo)：当前会话输入空闲及 tick 非单调限制。
17. Microsoft, [`WM_WTSSESSION_CHANGE`](https://learn.microsoft.com/en-us/windows/win32/termserv/wm-wtssession-change)：会话锁定、解锁、连接状态通知。
18. Microsoft, [`GetClipboardSequenceNumber`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getclipboardsequencenumber)：内容改变时递增及延迟呈现限制。

### SQLite

19. SQLite, [Atomic Commit](https://www.sqlite.org/atomiccommit.html) 与 [Write-Ahead Logging](https://www.sqlite.org/wal.html)：事务原子性、WAL 和同步/断电边界。
20. SQLite, [`PRAGMA integrity_check` / `foreign_key_check` / `secure_delete`](https://www.sqlite.org/pragma.html#pragma_integrity_check)：结构检查范围、外键需单独检查、删除覆写限制。

---

**M0 结论：**以上是供审查的契约，不是生产实现。审查通过并完成 Codex 公共接口协调后，才进入 M1；真实生物识别、跨平台原生与 GUI 验收均仍是后续门槛。
