# Antigravity：功能 07 全局划词翻译

模型保持 Gemini 3.8 Flash。专用 worktree：`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/antigravity-translator`；分支 `codex/feature-translator`；基线 `4689c49`。先核验 pwd、分支和 HEAD。只在此目录写入与执行命令；旧屏幕注释任务已结束，不再修改旧分支，不 merge/push。

阅读 `PROJECT_PLAN.md` 5.7、`docs/MODULE_ARCHITECTURE.md`、`docs/development/REMAINING_FEATURES_PLAN_2026-10-25.md`。首批实际完成 07 M1 中可独立实现的闭环：

- `mod-translator` 只依赖注入 trait；原生 AX/UIAutomation 读取选区另分平台文件。读取前保留目标应用焦点，不先弹气泡抢焦点。权限拒绝/不支持/无选区分别处理，不虚构文本。
- 显式快捷键/按钮触发，气泡及主面板、配置、三语言、开关生命周期。网络/原生等待不能阻塞 UI；关闭模块取消请求、监听与窗口，迟到结果不得覆盖新结果。
- 官方 Google/DeepL/OpenAI 兼容 provider 适配与 SSE 分片/UTF-8 边界、超时、取消和错误。缺 API Key 时如实提示；不要找用户现有密钥，不擅自外发选中文本、不自动购买服务。真实请求测试只在已有明确授权且配置可用时用专门的无敏感文本，其余标未测。
- API Key 不明文写 config/SQLite/日志，定义注入凭据存储接口；使用经审查的原生凭据存储或暂禁用持久化，不把接口替身冒充生产实现。
- 共享剪贴板租约由 Codex 统一实现。当前 EventBus 仅有 AtomicBool，并发不安全：你不得修改 EventBus/模块 02 公共实现。定义所需端口和错误契约，模拟复制降级保持显式禁用/Unsupported，先完成原生取词路径。缺公共能力不阻塞独立工作。
- 全局鼠标钩子、多源扩展、模拟复制属于后续 M2。保留待办，不伪造完整实现。新增窗口监听权限限定自己标签，不扩大核心权限。
- 配置、测试、业务分开，中文注释；不触碰其他成员模块或重构整个宿主。列清公共接线变化。
- 测试可用显式本地 HTTP fixture 验证协议，但不冒充真实外部翻译；覆盖并发/取消/停用/无权限。前端严格构建、Rust 模块及工作区测试，编制 GUI 用例，未实测项明确标记。
- 分批提交，正文记实际智能体/模型；交接 `docs/handoffs/translator-m1.md`，列提交、能力/限制、接口、公共文件、真实测试与待协作项。

先回复工作区/分支核验与实施步骤，然后立即开发，不只输出计划。
