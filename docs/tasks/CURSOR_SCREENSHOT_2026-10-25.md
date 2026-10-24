# Cursor 开发任务：01 增强截图与贴图，M1

负责人：Cursor Agent，用户指定 Claude Opus 5.5。当前只启动本任务，其他功能不在本次开发范围。

## 工作区与基线

- 主仓库：`/Users/santa_antilles/Desktop/your_all_in_one`。
- 专用 worktree：`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/cursor-screenshot`。
- 专用分支：`codex/feature-screenshot`，应包含 `f0db53d` 及本任务说明。
- 首先执行 `pwd`、`git branch --show-current`、`git rev-parse HEAD`；路径/分支不符时先切入专用 worktree，绝不在其他仓库或主工作区写代码。不要 reset、stash、删除用户改动。
- 先读 `PROJECT_PLAN.md` 5.1、`docs/MODULE_ARCHITECTURE.md`、`docs/development/REMAINING_FEATURES_PLAN_2026-10-25.md`、`DEVELOPMENT_CHECKLIST.md`。

## 首批交付

完成全计划 01 的 M1：真实冻结截图 → 手动/智能选区 → 编辑 → 保存/复制 → 独立贴图，接入总览开关、快捷键、主面板、三语言配置、按需窗口生命周期。

1. `crates/mod-screenshot` 只注入所需 HAL trait，不依赖 `NativeOsHal`、Tauri 或 `platform.rs`。业务、配置、测试分开，新增逻辑写清中文注释。
2. 真实冻结帧先于遮罩，显式处理逻辑/物理坐标、Retina 与屏幕边界。复用 `app-core::screenshot_region`，不重写现有同类算法；不可用的控件级探测如实显示，不能虚构窗口树。
3. 八手柄/键盘/Space 选区调整，矩形/箭头/马赛克/序号/文字标注，撤销重做，放大镜和取色。图像大帧不走庞大 JSON/base64 往返；设计并记录二进制 IPC/受控资源方案。
4. 真实图片保存/复制；文件独占创建防覆盖；贴图拖动、10%–500% 缩放、透明度、双击折叠、关闭销毁。
5. 捕获窗口、标注控件、贴图的层级可交互，参考 `src-tauri/src/ink_window.rs` 的已修复层级问题；不能只用 always_on_top 就假定工具栏不被画布盖住。Esc/停用/退出清理所有自有窗口和任务。
6. 按说明书安排快捷键并进行冲突处理；系统注册失败要回滚并报错，不能假报成功。
7. 原生能力缺权限必须返回真实错误和可操作提示，不自动授予权限或修改系统安全设置。

OCR、完整 L1/L2/L3 智能探测、ONNX 自定义模型、长截图、GIF 是后续 M2/M3，不在本次伪造空实现；在 HANDOFF 列明待办，不把 M1 标成整个模块完成。

## 协作边界

- 不改主工作区、其他 worktree、不 merge 或 push。不得破坏 `f0db53d` 中工具栏层级与事件权限修复。
- 可以在本分支完成自己的公共接线增量；不要重构整个 `tauri_app.rs`、覆盖其他模块字典或更改其他业务。
- 优先使用已有依赖；新增依赖须查官方文档、说明用途，不全局安装运行时或修改代理/审批设置。
- 独立前端依赖目录；若重用已有 node_modules 遇 pnpm worktree 校验，先记录并采用本目录合法构建方式，不更改其他目录依赖。
- 测试只用临时数据/自制图，不读取日常数据库、个人文档或系统秘密。没有真实 GUI 时明确未执行。

## 验收与交接

- 单测：裁剪/缩放/负坐标边界、选区几何和序列化、编辑操作、文件不覆盖。
- 集成：停用拒绝 IPC、初始化不抓屏、取消释放帧、关闭销毁、配置与升级数据保留、快捷键失败回滚。
- `cargo fmt --all -- --check`、相应模块测试、工作区离线测试（有新依赖时如实记录下载与失败）、前端 `build`（含 vue-tsc）与 `test`。
- 编制独立 GUI 用例：跨缩放裁剪、拖选/键盘调整、标注后再次操作工具栏、贴图缩放透明度、Esc/关闭模块/重启、权限拒绝。
- 分批 commit，正文写实际模型/智能体信息。在 `docs/handoffs/screenshot-m1.md` 记录实现范围、提交、公共文件清单、所有真实测试结果、未执行项和平台限制。
- 第一条回复先报告实际工作路径、分支、模型与 M1 实施步骤，然后开始实现。额度或权限阻塞时明确停在何处，不擅自换模型、不购买额度。
