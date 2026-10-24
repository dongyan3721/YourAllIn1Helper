# WorkBuddy：功能 10 密码本，先交 M0 设计

模型保持 Kimi K3.1 Pro。专用 worktree：`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/workbuddy-password-vault`；分支 `codex/feature-password-vault`；基线 `4689c49`。首先核验 pwd、分支和 HEAD。只在此 worktree 工作，不改旧媒体分支、主目录或其他 agent 目录，不 merge/push。

阅读 `PROJECT_PLAN.md` 5.10、`docs/MODULE_ARCHITECTURE.md`、`docs/development/REMAINING_FEATURES_PLAN_2026-10-25.md`。本轮首批验收是 M0 密钥/数据格式/接口设计，完成后提交报告交 Codex 审查；未审查前不开始生产密码学、认证/钥匙串写入或真实秘密测试。可以完成接口和测试方案文档，但不挂载空壳模块冒充可用。

请产出 `docs/design/password-vault-m0.md`，具体回答：

1. 随机 DEK、主密码 Argon2id KEK、包装信封、KDF 参数与升级、盐/nonce 随机源、密文版本。比较每字段信封和整条记录加密，明确 AAD 绑定记录 ID/字段/版本，绝不照抄单 entry_nonce 后用于多个字段。
2. Touch ID/Hello 认证结果与 Keychain/DPAPI 的实际密钥保护边界。说明注册、封装、解封、取消/不可用/设备凭据变化、主密码恢复；禁止“认证成功 bool + 普通可读文件中的 DEK”的绕过方案。官方资料与实现依赖需可追溯。
3. 锁定态 IPC、最小敏感返回、前端揭示值清除、后端 Zeroizing、180 秒真实用户空闲、锁屏/睡眠/离开页面/关闭/停用立即锁定与竞态。
4. 数据格式/SQLite 迁移/原子更新/损坏与篡改拒绝、现有六模块升级隔离。搜索哪些字段可明文、哪些必须解密，写清隐私取舍。
5. Codex 管理共享剪贴板服务。提出所需 trait/所有权 token/30 秒“仍为本次内容才清除”契约、与翻译租约并发和历史屏蔽。不得修改 EventBus/模块 02 公共代码或使用当前 AtomicBool 假装解决并发。
6. 威胁模型、内存清零保证边界、主密码错误/nonce 重用/篡改/AAD 记录交换/自动锁/复制竞态/崩溃恢复测试矩阵；单测、原生集成和 GUI 证据分开。
7. M1 主密码闭环、M2 生物识别及跨平台的具体实施清单、依赖、IPC 与配置约定、需要协调者决策的事项，不隐瞒待验收平台。

不读取现有系统密码、浏览器密码、个人凭据或用户数据库。此任务只用虚构测试示例；不触发系统生物识别弹窗，不更改安全配置。

设计完成后 commit（正文记录实际智能体/模型），再写 `docs/handoffs/password-vault-m0.md` 包含提交、推荐方案、待审查点和下一阶段，回复“等待设计审查”。这是首批设计门槛，不把密码本标为已实现。先确认路径/分支/模型，再开始实际写文档。
