# Windows 媒体、窗口置顶、自定义 LLM 实测（2026-11-11）

## 环境与结论

在本机 Windows 桌面进行实际 GUI 操作，使用真实 NativeOsHal、FFmpeg 和远端 LLM，未用模拟翻译结果。本轮使用隔离数据目录；日常数据和翻译服务设置不作变更。

| 功能 | 实测结果 |
| --- | --- |
| 媒体选择文件 | 发现并修复 PowerShell 7 下 WinForms 类型编译失败；替换为本程序持有的原生 IFileOpenDialog，重新选择视频成功 |
| 媒体选择输出目录 | 原生目录框选择含中文、空格的 D 盘目录成功 |
| 取消文件选择 | Escape 返回主界面，无错误、无新增文件 |
| GUI 视频裁剪 | 提交、运行、完成、100% 进度与速度显示正常；输出完整解码成功 |
| 真实处理队列 | 精确裁剪、MKV 转换、MP3 提取、M4A 音量调整成功 |
| 窗口置顶 | 自建记事本测试标签页使用 Ctrl+Shift+P 置顶，主窗口前置时仍被记事本遮挡；再次按快捷键解除后主窗口可正常覆盖记事本 |
| 自定义 LLM 配置 | GUI 添加服务源、选择服务源、保存会话密钥成功；端点使用 base URL，适配器自动补全 chat/completions |
| 缺少密钥 | GUI 明确提示未配置，无模拟结果 |
| LLM 普通/流式返回 | 通过实际 TranslatorService 分别发起 JSON 和 SSE 请求，均收到正确中文，分片拼接与最终结果一致 |
| GUI LLM 翻译 | 英文样句得到“会议将于明天上午九点开始。” |
| 外部划词翻译 | 记事本选中文字后 Ctrl+Shift+T，浮窗准确读取选区并得到“Windows 置顶 QA - 虚构内容。” |

## 媒体样本与输出

用户指定目录中的 `030.mp4`：8,818,910 字节，166.847667 秒，H.264 1280×720 与 AAC。测试前后原文件 SHA-256 相同：

```text
8f80ba56471d421078fa3db0b7f2f681624cd2955de974ca392355b4ab60d38d
```

复用本机已有的 FFmpeg/FFprobe：`C:\Program Files\ffmpeg-2026-06-10-git-b29bdd3715-full_build\bin`，版本 `2026-06-10-git-b29bdd3715-full_build-www.gyan.dev`。

GUI 从 0.1 到 3.1 秒精确裁剪，输出 `D:\YourAllIn1Helper-qa\2026-11-11\媒体 输出\030_media.mp4`，FFprobe 确认 3.000 秒、H.264 1280×720、AAC；FFmpeg 全片解码无错误。

后端真实队列先从原片裁剪 1–4 秒，再对这个短片转换、提取和调整音量，避免重复处理整段视频。输出保存在上述目录的 `backend` 子目录；MKV 3.011 秒，MP3/M4A 3.008 秒，均重新探测并完整解码。输出重名会加序号，原片不被覆盖。另执行既有合成样本集成测试，覆盖快速裁剪与精确裁剪等操作。

音量调整设置 0.25 倍，对输出与输入短片分别解码 PCM，测得 RMS 比例 **0.250**。

## 自定义 LLM 与密钥

使用用户授权的 `.env` 中 `LLM_API_KEY`、`LLM_BASE_URL`、`LLM_MODEL`。端点主机 `maas.qianwenaiapi.com`，模型 `qwen3.8-flash`。共四次真实翻译请求（普通、SSE、主界面、外部选区）；只发送虚构测试文本。

GUI 以 `--ui-smoke-test --keep-open` 运行，翻译密钥存储为会话内存；未将测试密钥写入日常 Windows 凭据条目。录入前关闭隔离实例的剪贴板采集。检查本轮 15 个配置、数据库及日志文件，未发现该密钥明文；退出隔离实例清空会话。此次未以真实密钥测试跨重启的系统凭据持久化。

## 修复与自动化复现

`native_media_windows.rs` 在专用 STA 线程内创建 IFileOpenDialog，以 Dashboard HWND 为 owner。文件模式支持多选和媒体过滤，目录模式使用 FOS_PICKFOLDERS；取消返回原有空选择 IPC 结果。无 PowerShell 依赖，无 stdout 路径编码问题。macOS 路径保持原实现。

新增真实服务/真实视频的显式忽略测试，普通测试不会自行读取用户文件或访问付费 API。显式运行时提供授权路径：

```powershell
# 先将已有 FFmpeg 的 bin 加入本进程 PATH。
cargo test -p all-in-one-host --lib native_dialog --locked
cargo test -p mod-media-proc --test real_ffmpeg --locked

$env:ALL_IN_ONE_QA_MEDIA_INPUT = '<授权视频的绝对路径>'
$env:ALL_IN_ONE_QA_MEDIA_OUTPUT = '<专用输出目录>'
cargo test -p mod-media-proc --test real_ffmpeg --locked authorized_real_video -- --ignored --nocapture

$env:ALL_IN_ONE_QA_LLM_ENV_FILE = '<授权 .env 的绝对路径>'
cargo test -p mod-translator --test live_custom_provider --locked -- --ignored --nocapture
```

原生对话框测试覆盖真实 COM 对象的模式设置、取消和错误分类；不弹窗。真实视频测试还检查音量输出的 PCM RMS 比例符合 0.25 倍设置，以及原文件逐字节未变。真实 LLM 测试检查中文、时间内容、UTF-8、请求 ID 和分片结果。

## 范围与剩余事项

此轮核心流程通过不等于全部格式、编解码器、长时间批处理、多显示器、提升权限窗口均已覆盖。窗口置顶页仍有 macOS 快捷键符号文案，Windows 实际快捷键以总览里的 Ctrl+Shift+P / Ctrl+Alt+P 为准；长进程路径也会使候选表格横向溢出。翻译自动复制和本地历史在界面明确禁用，尚未实现，未算作已通过功能。

本轮详细日志位于忽略目录 `.runtime_data/windows-validation/functional-2026-11-11/`，媒体输出位于 D 盘专用 QA 目录。

## 回归结果

- `cargo test --workspace --all-targets --locked -j 2`：34 个测试程序，350 通过、0 失败、18 忽略。真实 FFmpeg 合成样本测试在已配置 PATH 下实际执行。
- 显式真实视频测试：1 通过；真实 LLM 普通与流式联合测试：1 通过。
- `pnpm --dir src-ui test`：85 通过、0 失败。
- `pnpm build:desktop`：成功；修复后原生 UI smoke（窗口、模块开关、清理、还原、快捷键）：PASS。
