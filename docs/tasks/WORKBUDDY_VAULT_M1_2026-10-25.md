# WorkBuddy：功能 10 密码本 M1（主密码闭环）

协调者：Cursor Agent / Claude Opus 5.5（接替 OpenAI Codex 统筹）。执行者模型保持 **Kimi K3.1 Pro**；界面模型不符时先校正，不擅自更换模型、不购买额度。

## 工作区

- 专用 worktree：`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/workbuddy-password-vault`
- 分支：`codex/feature-password-vault`；基线为已合入第二轮集成的本地 master（本任务书所在提交）。
- 先执行 `pwd`、`git branch --show-current`、`git rev-parse HEAD` 并报告。只在本 worktree 工作；不改主目录、其他成员目录，不 merge/push。使用本 worktree 独立 `target/` 与 `src-ui/node_modules`。

## 先读（必须遵守）

- 你的设计：`docs/design/password-vault-m0.md`
- **审查决定（8 条约束，冲突时以此为准）**：`docs/design/password-vault-m0-review.md`
- `docs/MODULE_ARCHITECTURE.md`、`PROJECT_PLAN.md` 5.10、`docs/testing/ROUND2_INTEGRATION_REPORT_2026-10-25.md`
- 参考已集成模块的结构：`crates/mod-app-usage`、`crates/mod-screenshot`（配置/业务/测试分离、生命周期、IPC 接线方式）。

## 本轮实际开发内容（M1）

1. `crates/mod-password-vault`，模块 ID `password_vault`，实现 `AppModule`，可重复启停。
2. 密码学（v1 冻结格式）：OS CSPRNG 随机 DEK；`argon2id-v19-m65536-t3-p4` 主密码 KEK 包装 DEK；每字段独立 AES-256-GCM 信封、独立随机 nonce、规范 AAD 绑定（记录 ID / 字段名 / 格式版本）；长度/字段/AAD 严格校验；CSPRNG 失败明确报错。锁定具体 RustCrypto crate 版本并在交接中写明。KDF 并发限制为 1，放到阻塞线程，不阻塞 UI 与 AppKit 主线程。
3. 存储：新增空表迁移（不触碰旧模块表），记录更新重加密所有现存字段，事务提交与版本检查不可拆开；损坏、篡改、AAD 记录交换必须拒绝；无自动修库、无自动导入。
4. 生命周期与锁定：初始化 / 解锁 / 锁定；锁定态拒绝所有敏感 IPC；后端 `Zeroizing` 管理密钥；180 秒**真实用户**空闲锁定（不得用后台轮询刷新空闲时间）；锁屏/睡眠/离开页面/关闭主面板/停用模块立即锁定；锁定 epoch 丢弃迟到结果；原生能力查询失败时按锁定处理；主密码变更成功后强制重锁。
5. 功能：增删改查、搜索（写清哪些元数据明文、哪些需解密）、密码生成器；前端揭示值短驻留，锁定/离开/关闭时清除。
6. **复制秘密：本轮保持禁用并在界面说明原因**（审查第 6 条）。协调者正在实现带所有权的共享剪贴板租约与「仍为本次内容才清除」服务，完成审查后由协调者接入，不要自行用 AtomicBool 或文本比较替代。
7. **生物识别：M1 明确不可用**，界面禁用并说明；不触发 Touch ID / Hello 弹窗，不写钥匙串。
8. 主面板 `/vault` 页签与三语字典（只新增 `vault.*` 键，不覆盖其他字典）；在 `src-tauri/src/modules.rs` 挂载并经 `with_enabled` 路由。

## 测试与证据（只用虚构数据）

- 单测：已知向量、错误主密码、篡改密文、AAD 交换、nonce 重复与 CSPRNG 失败注入、锁定竞态与迟到结果、锁定态 IPC 拒绝、停用清理、重启后密文完整、迁移不影响旧模块数据。
- `cargo fmt --all -- --check`、`cargo test --workspace --offline --all-targets`、`pnpm --dir src-ui run build`、`pnpm --dir src-ui test`。允许从 crates.io 官方源下载所需依赖（argon2、aes-gcm、zeroize、rand_core/getrandom 等），如实记录下载。
- GUI 用例写入 `docs/testing/GUI_TEST_PASSWORD_VAULT.md`，未实际操作的全部标「未执行」（协调者负责 GUI 验收）。
- 不读取任何系统密码、浏览器密码、个人凭据或日常数据库；不更改安全配置。

## 交付

- 分批 commit，正文记录 `Agent: WorkBuddy` 与 `Model: Kimi K3.1 Pro`。
- 交接 `docs/handoffs/password-vault-m1.md`：提交、格式/迁移/IPC/配置、公共文件清单、三层证据（单测 / 原生 / GUI）、未执行项、明确的安全边界（不声称全进程物理清零）。
- 完成后回复「等待 M1 安全审查」。审查通过前协调者不会合入 master，不得使用真实秘密。

先确认路径、分支、模型与实施步骤，然后开始编码。阻塞时报告具体原因，不伪报进度。
