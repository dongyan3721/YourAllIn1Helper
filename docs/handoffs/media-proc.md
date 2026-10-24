# HANDOFF — 功能 08 基础多媒体处理

## 提交信息

- 独立 worktree：`/Users/santa_antilles/Desktop/your_all_in_one/.runtime_data/team/workbuddy-media-proc`
- 分支：`codex/feature-media-proc`
- 基线：`68fecea028a3bce8cd6d0b3975919441355c793c`
- 功能实现提交：`44d64789b7060d334cd7db68a49455404681b05c`（`feat(media): add image and ffmpeg processing queue`）
- 本文件为后续独立交接提交；未 merge、未 push、未修改主目录或其他成员 worktree。

## 已实现

### Rust 媒体模块与配置

- 新增 `crates/mod-media-proc`，模块 ID `media_proc`，实现 `AppModule`，可重复启停。
- 图片使用真实 `image` Rust 编解码器：PNG、JPEG、WebP、BMP、ICO；支持 JPEG 1–100 质量、宽高缩放和保持比例。WebP 为真实无损编码，不虚标质量选项；未宣称 AVIF/TIFF。
- 批量任务、1–4 并发限制、输出目录与历史容量配置；配置通过 `modules.settings.media_proc` 和 `kernel.update_config` 原子持久化，未知设置字段保留。
- 新增 `media_task_history` SQLite 迁移及任务历史持久化；重启保留历史，未完成旧任务标为取消，历史清理不会删除活动任务。
- 输出使用独立 `_media` 名称和序号；活动输出名全程预留，最终 hard-link 提交防止覆盖并发出现的外部文件；Unicode、空格路径和扩展名均按 `Path` 处理。取消/失败清理 `.partial` 文件。

### FFmpeg / FFprobe 与生命周期

- 仅通过 `EnvProbeService` 探测系统或用户配置的 FFmpeg/FFprobe；没有自动安装、下载或打包运行时。
- 实现 FFprobe JSON 媒体信息、音视频格式转换、快速/精准时间裁剪、提取音频、音量调整；全部使用参数数组，无 shell 拼接。音量调整明确使用 `-vn`，纯音频输出不会残留视频流。
- 消费 `-progress pipe:1` 的 `out_time_us`、`speed`、`progress=end`；裁剪使用裁剪区间时长计算进度。
- 模块自有后台线程，不阻塞同步 IPC；并发信号量、64 KiB stderr 有界日志、FFprobe 输出上限、任务取消、子进程 kill/reap、线程 join、停用清理均已实现。
- 子进程发布后再次检查取消标志，关闭 spawn/publish 竞态；图片解码、缩放、编码、flush、落盘之间设置取消检查点。
- 原生进度事件：`media://job-progress`，载荷含 `job_id`、`progress`（0–100）、`speed`、`status` 和完整 `job`；前端另有轮询兜底。

### Host / IPC / UI

- Host 注册同一个 `MediaProcModule` Arc，路由全部经过 `with_enabled("media_proc")`，禁用后拒绝媒体 IPC。
- IPC：`media_check_ffmpeg_env`、`media_inspect_file`、`media_convert_images`、`media_submit_av_job`、`media_cancel_job`，以及 `media_list_jobs`、`media_get_config`、`media_update_config`、`media_pick_files`、`media_pick_directory`。
- macOS 使用系统 osascript 文件/目录对话框，Windows 使用 PowerShell WinForms；不引入大型依赖。
- 主控台新增媒体页：选择/拖拽、批量待处理列表、输出设置、图片与音视频参数、环境提示、队列进度/状态/失败原因、取消、媒体信息 JSON。
- 支持 `/media` 首次路由及已打开 Dashboard 的 `dashboard://navigate` 切换。
- 简体中文、English、日本語字典及共享 IPC 类型已接入。

## 真实测试结果

最终验收在当前分支源码上执行；安装 Homebrew FFmpeg 后补充了真实运行时集成测试：

1. `CARGO_TARGET_DIR=/Users/santa_antilles/Desktop/your_all_in_one/target cargo test --workspace --offline --all-targets`
   - **通过**：34 passed，0 failed，1 ignored。
   - ignored 为既有 `test_native_windows_list`，要求登录桌面会话，不属于媒体测试。
   - 覆盖真实五种图片编码/解码与缩放、参数验证、输出冲突、SQLite 重启持久化、禁用拒绝 IPC、并发限制、失败原因、取消和停用清理。
   - 进程控制测试使用测试目录内明确标注的测试专用可执行替身，仅验证生产进程管理逻辑，不冒充真实 FFmpeg 处理能力。
2. `cargo fmt --all -- --check`
   - **通过**。
3. `pnpm --config.verify-deps-before-run=false --dir src-ui run build`
   - **通过**：`vue-tsc --noEmit` 与 Vite 生产构建成功。
   - 额外配置仅用于 pnpm 11 对符号链接 `node_modules` 的自动依赖校验；没有安装新依赖。
4. `pnpm --config.verify-deps-before-run=false --dir src-ui test`
   - **通过**：2 passed，0 failed；三语言键集合一致，Logo 检查通过。
5. `command -v ffmpeg && command -v ffprobe && ffmpeg -version && ffprobe -version`
   - **通过**：`/opt/homebrew/bin/ffmpeg` 与 `/opt/homebrew/bin/ffprobe`，版本均为 9.0.2；运行时由用户通过 Homebrew 安装，模块未执行安装。
6. `CARGO_TARGET_DIR=/Users/santa_antilles/Desktop/your_all_in_one/target cargo test -p mod-media-proc --test real_ffmpeg --offline -- --nocapture`
   - **通过**：1 passed，0 failed；使用 FFmpeg 临时合成 2 秒无版权音视频样本，通过模块完成 FFprobe 探测、MKV 转换、快速/精准 MP4 裁剪、MP3 音频提取、M4A 音量调整。
   - 所有最终输出均再次由真实 FFprobe 解码并校验流类型；精准裁剪时长为 1 秒（容差 0.25 秒），音量输出仅含音频流；事件包含 0–100 进度和速度，且没有遗留 `.partial` 文件。
   - 首次实测发现音量调整保留视频流的问题，已在生产命令中补 `-vn` 并回归通过。测试源码为 `crates/mod-media-proc/tests/real_ffmpeg.rs`，在未安装运行时的机器会明确打印未执行并返回，不伪造运行结果。

## GUI 状态与缺口

- **GUI 未执行**：未进行真实桌面点击、原生拖拽、文件对话框、窗口重启、主题切换或播放验证，不能把 IPC/单测视为 GUI 通过。
- 功能专属用例在 `docs/testing/MEDIA_GUI_TEST_PLAN.md`，12 项均明确标记未执行；后续实测需补构建 SHA-256 与截图。
- 真实 FFmpeg/FFprobe 后端链路已验证；GUI 内的音视频操作、事件展示及输出播放仍未执行，不能以集成测试替代。
- 图片取消在流水线检查点生效；单个非常大的 Rust 编码调用无法像外部子进程一样被强制中途 kill，停用会等待该调用返回后再完成清理。

## 修改的公共文件

- `Cargo.toml`、`Cargo.lock`
- `config/config.example.json`
- `docs/MODULE_ARCHITECTURE.md`、`docs/testing/GUI_TEST_PLAN.md`
- `src-tauri/Cargo.toml`
- `src-tauri/src/lib.rs`、`src-tauri/src/modules.rs`
- `src-tauri/src/ipc/mod.rs`、`src-tauri/src/tauri_app.rs`
- `src-tauri/tests/modules.rs`、`src-tauri/tests/unit/lib.rs`
- `src-ui/src/api/ipc.ts`、`src-ui/src/components/Dashboard.vue`
- `src-ui/src/locales/zh-CN.json`、`en-US.json`、`ja-JP.json`

功能独立新增：`crates/mod-media-proc/**`、`src-tauri/src/ipc/media.rs`、`src-ui/src/components/MediaPanel.vue`、`docs/testing/MEDIA_GUI_TEST_PLAN.md`。

## 集成注意事项

- 本分支将已挂载模块数从 3 更新为 4；与其他功能分支集成时需合并目录数量及测试预期，而不是简单覆盖。
- `tauri_app.rs` 同时包含媒体进度事件桥和 Dashboard 路由事件；解决公共文件冲突时保留这两段。
- `Dashboard.vue` 的媒体 tab 依赖 `MediaPanel.vue`；三语 `media` 根键与 `tabs.media` 必须一起保留。
- `src-ui/node_modules` 是按要求创建的根目录依赖符号链接，已被 git 忽略，不包含在提交中。
