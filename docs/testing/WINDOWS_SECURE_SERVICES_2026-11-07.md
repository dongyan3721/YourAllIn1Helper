# Windows 安全服务迁移（SecretStore / 敏感剪贴板 / GIF 剪贴板）执行报告（2026-11-07）

分支：`codex/windows-secure-services`（worktree `YourAllIn1Helper-windows-secure-services`，基线 `c7c6a4d`）。
负责范围：`crates/os-hal/src/platform/secure.rs` 及拆分子实现、`crates/os-hal/src/secure.rs` 契约文档与相关测试；
经集成负责人预审授权，另对 `src-tauri/src/tauri_app.rs` 的剪贴板监听器做了最小接线。

## 实现

### 1. Windows SecretStore（凭据管理器真实读写删）

- 新增 `crates/os-hal/src/platform/secure/secret_win.rs`：`CredReadW` / `CredWriteW` / `CredDeleteW` / `CredFree` 真实调用。
- 条目限定在本应用命名空间内：`service` 必须以 `com.allinone.desktop` 开头，TargetName 为 `{service}/{account}`；
  拒绝空片段、`/`（分隔符歧义）与 NUL（C 字符串截断）；TargetName 超过 32767 UTF-16 单元明确报错。
  命名空间外的 service 直接拒绝，绝不触碰用户已有的其他凭据。
- 通用凭据 `CRED_TYPE_GENERIC` + `CRED_PERSIST_LOCAL_MACHINE`（仅本机持久化，不随 Microsoft 账户漫游）。
  **这是普通用户凭据存储，不是 Windows Hello/生物识别保护，也没有标记为受保护的设备密钥**；
  设备密钥保护是密码本模块的独立需求，不在本实现中冒充。
- 秘密按二进制 blob 原样存取（`CredentialBlob`），不假定 UTF-8；上限 `CRED_MAX_CREDENTIAL_BLOB_SIZE`（2560 字节），超限明确报错。
- 错误路径：`ERROR_NOT_FOUND` 读取→`Ok(None)`、删除→`Ok(())`；`ERROR_ACCESS_DENIED`→`PermissionDenied`（带指引）；
  其余→`NativeCallFailed`（含动作与错误码）；错误信息不含秘密内容。

### 2. Windows ClipboardOwnership（敏感写入 / 序号所有权 / 条件清除）

- 新增 `crates/os-hal/src/platform/secure/clipboard_win.rs`。
- **有效 owner 窗口**：OpenClipboard 文档明确 NULL owner 时 `EmptyClipboard` 后 `SetClipboardData` 会失败；
  因此每次事务在本线程用系统预定义 `STATIC` 类 + `HWND_MESSAGE` 父窗口创建纯消息窗口作 owner，
  同一线程创建与销毁（RAII），不依赖主控台存在、不使用其他程序窗口。
- 敏感写入：`CF_UNICODETEXT`（UTF-16 含 NUL 终止；含 NUL 字符的文本拒绝写入，避免读者截断）；
  同时写入三个微软文档《Clipboard Formats》→ Cloud Clipboard and Clipboard History Formats
  小节给出的准确注册格式名（**无空格**，预审修正，见下方「预审修正记录」）：
  - `ExcludeClipboardContentFromMonitorProcessing`（总控：剪贴板历史与云同步均排除）；
  - `CanIncludeInClipboardHistory = 0`、`CanUploadToCloudClipboard = 0`（序列化 DWORD，细粒度双保险）。
  任一标记写入失败即回滚 `EmptyClipboard` 并报错，绝不留下未隐藏的秘密。
- 序号所有权：凭证只记 `GetClipboardSequenceNumber`，不保存内容。实测 `CloseClipboard` 才把本事务的
  变更最终计入序号（关闭前读到的是暂态值），因此**序号在关闭后采集，但只在证明仍属本次写入后采集**：
  写入时附加本应用私有唯一标记（`com.allinone.desktop.ClipboardWriteProbe`，载荷为「进程号+自增计数」
  拼成的 8 字节值），事务关闭后用**仍存活的本次 owner 消息窗口**重新独占打开，验证
  （a）`GetClipboardOwner` 仍是本次窗口、（b）私有标记值仍等于本次写入，验证通过才在该独占会话内
  采集最终序号；被其他程序抢占（含复制相同文本）则安全放弃——不产出凭证、不触碰剪贴板，
  绝不认领或清除他人内容。序号 `0` 不是可靠所有权：写入路径拒绝产生，且其失败收尾只在
  验证通过的独占会话内清空（只可能删掉本次内容，绝不会误清后来的复制）。
- `clear_if_unchanged`：先独占打开剪贴板再比较序号（检查与清空之间无竞态），序号变化（含用户复制相同文本）返回 `false` 且不触碰。
- 资源/句柄：`GMEM_MOVEABLE` 分配、锁定拷贝、解锁后 `SetClipboardData` 成功即所有权移交系统（不释放）；
  所有失败路径释放自有句柄，`CloseClipboard`/`DestroyWindow` 由 RAII 保证。
- `GlobalUnlock` 语义修正：「FALSE 且 last-error=0」是**正常完成解锁**（锁计数归零），windows-rs 的 `BOOL→Result`
  包装会把它变成 `Err`（且 `code` 被 `nonzero_hresult` 替换为 `S_EMPTY_ERROR`，不能假定 `code()==0`）。
  实现采用调用前 `SetLastError(0)`、失败分支读取原生 `GetLastError` 显式判定；实测真实失败
  （对已解锁句柄再解锁）为 `ERROR_NOT_LOCKED(158)`。
- `GlobalFree` 的 windows-rs 包装语义与直觉相反（成功返回 NULL→`Err`，失败返回句柄→`Ok`），代码按真实语义处理并有测试锁定。

### 3. 动态 GIF 的 Windows 剪贴板写入

- `write_animated_gif_clipboard`：空内容先行拒绝（与 macOS 同口径）；写入注册格式 `image/gif`（主）与 `GIF`（辅，尽力而为），
  对应 macOS 的 `com.compuserve.gif` / `public.gif` 双类型结构。同样使用有效 owner 窗口；GIF 是非敏感内容，不设排除标记。
- GIF 只写自定义格式、不写 `CF_DIB`：保持动画语义，与 macOS 实现（不写静态回退）一致；
  只接受位图粘贴的目标应用无法粘贴（与 macOS 同类限制，见未测项）。

### 4. 本应用剪贴板监听器接线（最小改动）

- 现状核查：监听器（`start_clipboard_listener`）经 `ClipboardModule::capture_text` 的
  `EventBus::is_clipboard_suppressed()` 判断收录，密码本复制（`copy_secret_with_lease`，35 秒租约）等敏感写入
  期间监听器不会收录；租约覆盖整个秘密停留窗口。
- 按预审授权补了最小接线：监听器文本与图片两通道在收录前调用新增的只读探测
  `os_hal::clipboard_content_is_concealed()`（Windows 查三个排除标记是否在剪贴板上；macOS 查 `org.nspasteboard.ConcealedType`），
  带标记内容不入历史。探测不打开、不修改剪贴板。
- 契约文档（`crates/os-hal/src/secure.rs`）明确：`write_concealed_text` 的调用方必须在敏感内容停留于剪贴板期间
  持有剪贴板屏蔽租约；排除标记 + 租约 + 探测三层防护职责在文档中固定。

### 5. 契约文档与 macOS 保留

- `crates/os-hal/src/secure.rs` 仅更新文档（含 Windows 凭据管理器语义、排除标记、序号凭证、监听器配合要求），
  trait 签名零改动，向后兼容。
- macOS 实现逐行保留（钥匙串、NSPasteboard、ConcealedType、changeCount）；新增的 macOS 敏感探测分支
  复用与既有代码相同的 objc2 API 形态，**未在本机（Windows 会话）编译验证 macOS 目标**。
- `os-hal/Cargo.toml` 仅增量：windows feature `Win32_Security_Credentials` / `Win32_System_DataExchange` / `Win32_System_Memory`
  （`GlobalFree`/`GetLastError` 等在已启用的 `Win32_Foundation`）；集成测试用 `windows` dev-dependency（同锁定版本 0.62.2）。
  **Cargo.lock 未改动**。

## 变更文件

| 文件 | 变更 |
| --- | --- |
| `crates/os-hal/Cargo.toml` | 增量 windows features + 测试用 windows dev-dependency |
| `crates/os-hal/src/platform/secure.rs` | Windows 分支分发、`clipboard_content_is_concealed` 门面、测试挂接 |
| `crates/os-hal/src/platform/secure/clipboard_win.rs` | 新增：Windows 剪贴板所有权与 GIF 写入 |
| `crates/os-hal/src/platform/secure/secret_win.rs` | 新增：Windows 凭据管理器 SecretStore |
| `crates/os-hal/src/secure.rs` | 契约文档更新（trait 未变） |
| `crates/os-hal/src/lib.rs`、`platform/mod.rs` | 导出 `clipboard_content_is_concealed` |
| `crates/os-hal/tests/unit/secure_windows.rs` | 新增：纯逻辑回归单测 + 2 项确定性抢占回归（真实剪贴板，`#[ignore]`） |
| `crates/os-hal/tests/secure_services.rs` | 新增：二进制秘密契约测试 + 5 项 Windows 系统集成（`#[ignore]`） |
| `src-tauri/src/tauri_app.rs` | 监听器最小接线（两通道收录前探测敏感标记） |

## 测试命令与结果

环境：Windows x64 25H2（Build 26200），Rust stable `x86_64-pc-windows-msvc`；
`CARGO_TARGET_DIR` 指向本 worktree 的 `target`，`.cargo/config.toml` 限制 `jobs = 2`。

| 命令 | 结果 |
| --- | --- |
| `cargo test -p os-hal --locked` | 通过：lib 16（另 3 项 ignored：1 项既有窗口枚举 + 2 项确定性抢占回归）、capabilities 1、secure_services 4（另 5 项 ignored）、native_ocr 0 |
| `cargo test -p os-hal --locked --test secure_services -- --ignored --test-threads=1` | 5 项系统集成全部通过（见下） |
| `cargo test -p os-hal --locked --lib -- --ignored --test-threads=1 windows_tests` | 2 项确定性抢占回归全部通过（见预审修正记录 4） |
| `cargo check --workspace --locked --all-targets` | 通过（3m43s；剩余警告为基线已有：平台分支未使用项、`mod-password-vault` 的 `KEYCHAIN_SERVICE` dead code 等，未跨范围处理） |
| `cargo test -p all-in-one-host --locked` | 通过：20 + 1 + 15 + 4 + 1 = 41 项，0 失败 |

最终集成审查修正轮（格式名之后）按约定只重跑 `os-hal` 测试（默认 + 两个 ignored 集成目标），
未再全 workspace 编译；此前两轮的 workspace 编译与宿主单测结果见上表。

普通单测（`tests/unit/secure_windows.rs` + mock 契约测试）不读写用户真实剪贴板/凭据、不弹权限框，覆盖：
UTF-16 转换与 NUL 拒绝、文档化标记格式名防漂移、命名空间约束与 TargetName 歧义/长度拒绝、2560 字节 blob 边界、
Win32 错误映射、空 GIF 拒绝（不触剪贴板）、`GlobalUnlock` 正常完成/真实失败分支、`GlobalFree` 反向包装语义、
owner 窗口创建销毁、二进制秘密（含 NUL 与非 UTF-8）往返、mock 序号所有权（含复制相同文本不误清）。

系统集成验证（显式 `-- --ignored`，全部使用本应用独立虚构命名空间 `com.allinone.desktop.test.secure-services`
与虚构剪贴板文本，串行执行；剪贴板用例会短暂覆盖系统剪贴板，结束时清空）：

| 用例 | 结果 |
| --- | --- |
| `native_credential_manager_round_trip_uses_dedicated_test_item`（二进制 blob + 非 ASCII 账户名 + 重复删除） | 通过 |
| `native_credential_manager_rejects_out_of_namespace_service` | 通过 |
| `native_pasteboard_clears_only_unchanged_content`（三标记齐备 + 探测一致 + 被改不清 + 过期凭证不清） | 通过 |
| `native_pasteboard_keeps_user_copy_of_identical_text`（复制相同文本不误清） | 通过 |
| `native_animated_gif_clipboard_write_registers_gif_formats`（主/辅格式字节一致、无排除标记） | 通过 |

开发中实测并修正的行为：`CloseClipboard` 会把本事务变更最终计入序号（关闭前采集凭证会永远失配）；
`GlobalUnlock` 对已解锁句柄返回 FALSE + `ERROR_NOT_LOCKED(158)`，正常完成为 FALSE + last-error 0；
`GlobalFree` 的 windows-rs `Result` 包装成功/失败语义反转。

## 预审修正记录

1. **排除格式名修正（阻断问题）**：初版误用了带空格的格式名（`Exclude Clipboard Content From Monitor Processing`
   等），Windows 并不认识。经官方文档（learn.microsoft.com/.../dataxchg/clipboard-formats，Cloud Clipboard and
   Clipboard History Formats 小节）核实，准确注册名为
   `ExcludeClipboardContentFromMonitorProcessing` / `CanIncludeInClipboardHistory` / `CanUploadToCloudClipboard`（均无空格），
   后两个为序列化 DWORD（0=排除、1=显式允许），载荷用法正确。已修正写入、
   `clipboard_content_is_concealed` 探测（自有监听路径自动跟随）、契约文档、格式名回归单测
   （含「注册名不得含空格」断言，不触真实剪贴板）与集成测试断言。
   修正后全部复测：`cargo test -p os-hal --locked` 通过（15+1+4）；
   `-- --ignored --test-threads=1` 5 项集成全过（真实剪贴板上断言到修正后的三个标记名）；
   `cargo check --workspace --locked --all-targets` 通过。
2. **owner HWND 审查项（已落实）**：OpenClipboard 以 NULL owner 打开后 EmptyClipboard 会使 SetClipboardData 失败——
   实现为每次事务在本线程创建系统 `STATIC` 类 + `HWND_MESSAGE` 纯消息窗口作 owner，同一线程 RAII 创建/销毁，
   不依赖主控台、不使用其他程序窗口；`owner_window_creates_and_destroys_within_the_calling_thread` 单测锁定。
3. **GlobalUnlock 审查项（已落实）**：「FALSE 且 last-error=0」为正常完成解锁，不误报失败；实现调用前
   `SetLastError(0)`、失败分支读取原生 `GetLastError` 显式判定（不假定被 `nonzero_hresult` 替换后的
   `Error.code()==0`）；实测真实失败为 `ERROR_NOT_LOCKED(158)`，两条分支均有单测锁定。
   顺带核实并按真实语义处理了 `GlobalFree` 的 windows-rs 反向包装（成功返回 NULL→`Err`）。
4. **最终集成审查修正（关闭后抢占竞态，阻断问题）**：关闭后直接采序号会把「抢占者的序号」记为凭证、
   其后误清他人复制，不可接受。已改为**验证式采集**：私有唯一标记 + 存活 owner HWND 双重证明
   「当前内容仍是本次写入」才采序号，抢占即安全放弃（见实现 2）；序号 0 的失败收尾同样只在
   验证通过的独占会话内清空。补了两项**确定性**回归（`#[ignore]`，通过测试钩子在
   「关闭后、采序号前」注入外部复制，不依赖概率）：
   `preemption_between_close_and_capture_is_abandoned_and_never_cleared`（不同文本抢占→放弃且外来内容原样保留）与
   `identical_text_copied_after_close_is_still_preemption_and_never_cleared`（同文本抢占→同样放弃，且旧凭证不清除用户之后的同文本复制）。
   两用例在开发中抓出并促成修复了 `ClipboardSession` 缺少 `CloseClipboard` Drop 的缺陷（会话未关闭会导致
   后续 `OpenClipboard` 拒绝访问）。trait 签名保持兼容；密码本 `copy_secret_with_lease` 的 HAL 接线按约定由集成负责人完成，本轮未越界修改。
   探测保守语义说明：`clipboard_content_is_concealed` 把 `CanIncludeInClipboardHistory` /
   `CanUploadToCloudClipboard` 的**存在**（即使 DWORD 值为 `1` 显式允许记录）也视为敏感——
   载荷值需打开剪贴板才能读取，探测刻意不打开（监听热路径），故可能跳过少量可记录内容，但绝不会收录秘密。

## 跨范围观察（不修改，交集成负责人）

1. `src-tauri/src/ipc/vault.rs::copy_secret_with_lease`（密码本复制，Cursor 范围）仍用 `arboard::set_text` + 30 秒后
   **文本等值**判断清除——用户复制相同文本会被误清，且未设置 Windows 剪贴板历史/云同步排除标记。
   建议迁移为 `ClipboardOwnership::write_concealed_text` + `clear_if_unchanged`（本分支已提供）。
2. `ClipboardOwnership` / `SecretStore` 目前生产调用方仅翻译凭据接线（`secure_credentials.rs` 已就绪，
   Windows 上 `is_available()` 现为 true，翻译 Key 将持久化到凭据管理器 `com.allinone.desktop.translator/{provider}`）。

## 未测项与已知限制

- **未测硬件/桌面项**：Windows 剪贴板历史（Win+V）与云剪贴板的排除效果需人工开启对应设置后肉眼验证
  （代码按微软文档化格式写入，自动化仅验证标记存在）；多会话/快速用户切换下的凭据可见性未测；
  macOS 目标未编译验证（本机仅 Windows 工具链）。
- GIF 粘贴端兼容性：只写 `image/gif`/`GIF` 注册格式，未写 `CF_DIB` 静态回退（保持动画）；
  仅接受位图的目标应用不能粘贴，需按目标应用人工验收。
- 关闭后抢占竞态：**已消除**（不再认领他人内容）——序号只在私有唯一标记 + owner HWND 验证通过的
  独占会话内采集，抢占即安全放弃，并有确定性回归测试锁定。唯一剩余的窄边缘是「第三方在我们的
  剪贴板项仍存活时、不经过 EmptyClipboard 直接追加一个格式」（这不是「复制」语义，用户的复制必经
  Empty+Set、必然被识别为抢占）：验证会视为仍是本次写入，其后 `clear_if_unchanged` 清空时会连同
  该追加格式一起删除。此用法极罕见且不丢用户复制内容，作为如实说明保留。
- 凭据管理器单条秘密上限 2560 字节（平台限制），更大秘密需调用方自行分块或改用其他机制，当前超限明确报错。
- 真实剪贴板集成测试会短暂覆盖系统剪贴板内容（运行前请勿依赖剪贴板中的内容）；已在用例标记与本报告声明。

## 提交

- 代码与测试提交：`cf3a222f3b82cd42e785d44bffc0ccffb869169c`
  （`feat(os-hal): Windows secret store, concealed clipboard ownership and GIF clipboard write`）。
- 预审修正提交（排除格式名）：`790445f895ef4ccaed5c1d03d30e4139a611b46a`
  （`fix(os-hal): use the exact documented clipboard exclusion format names`）。
- 最终集成审查修正提交（所有权验证式序号采集）：`249e3df60ba6d3ecd191613d80bcdaffb99285f3`
  （`fix(os-hal): capture clipboard ownership sequence only after proving the write is still current`）。
- 本报告在随后的文档提交中入库（仅含本文件；分支顶端哈希以 `git log -1 --format=%H` 为准，聊天回执中给出）。
- 未推送远程、未合并其他分支、未改写历史。
