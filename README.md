# All-in-One Toolbox

基于 Tauri 2、Rust 和 Vue 3 的桌面工具箱，面向 Windows 与 macOS。

## 项目结构

- `crates/app-core`：微内核、模块生命周期、SQLite、配置、快捷键和外部工具探测。
- `crates/os-hal`：截图、输入、OCR、窗口和应用使用状态等系统能力适配。
- `crates/mod-*`：截图、剪贴板、屏幕注释、便笺、应用统计、脚本、翻译、多媒体、窗口置顶、密码本十个功能模块。
- `src-tauri`：原生桌面宿主、托盘、多窗口和 IPC。
- `src-ui`：Vue 3 + TypeScript + Vite 界面。

各功能当前的实现范围以源码和实际运行结果为准；`PROJECT_PLAN.md` 是设计规划，不代表所有能力均已完成。

## Windows 开发环境

需要 Node.js 22.13+（推荐 24 LTS）、pnpm 10、Rust stable 的 `x86_64-pc-windows-msvc` 工具链、Visual Studio 2022 Build Tools 的“使用 C++ 的桌面开发”工作负载以及 WebView2 Runtime。

Tauri 官方依赖说明：<https://v2.tauri.app/start/prerequisites/>。

在 PowerShell 中安装构建工具和 pnpm：

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --source winget --override '--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended' --accept-source-agreements --accept-package-agreements
winget install --id Rustlang.Rustup --exact --source winget --accept-source-agreements --accept-package-agreements
npm install --global pnpm@10
```

等待安装结束后重新打开终端，确认工具链：

```powershell
node --version
pnpm --version
rustc --version
cargo --version
rustup show active-toolchain
```

如果安装 Rust 前已经打开了 IDE/Codex，需要重启 IDE/Codex 以刷新 PATH。当前 PowerShell 会话也可临时刷新：

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
```

## 安装依赖和启动

在仓库根目录执行：

```powershell
pnpm run setup
pnpm dev
```

`pnpm dev` 同时启动 Vite 和 Tauri 原生应用，前端固定使用 `http://localhost:5173`。首次 Rust 编译需要下载依赖并编译，耗时会较长。

构建可直接运行的调试版（不制作安装包，界面资源嵌入 exe）：

```powershell
pnpm build:desktop
.\target\debug\all-in-one-desktop.exe
```

`pnpm build:app` 制作当前平台默认的发布安装包；Windows 不使用 macOS 专属的 `--bundles app`。

应用启动后会打开主控台并常驻托盘。关闭主控台窗口会隐藏窗口，完全退出请使用托盘菜单。

## 验证

```powershell
pnpm --dir src-ui run build
pnpm --dir src-ui test
cargo test --workspace --locked --all-targets
pnpm build:desktop
.\target\debug\all-in-one-desktop.exe --smoke-test
.\target\debug\all-in-one-desktop.exe --ui-smoke-test
```

无桌面烟测使用模拟系统适配器；原生 UI 烟测会创建临时窗口、测试快捷键注册和截图生命周期。两者不等价于完整的人工功能验收。
测试后重新运行 `build:desktop`，确保烟测使用嵌入界面资源的版本，不依赖开发服务器。

手工界面测试可以隔离数据，但仍使用真实 Windows 系统能力：

```powershell
$env:ALL_IN_ONE_GUI_TEST_DATA_DIR = Join-Path $PWD '.runtime_data\windows-gui'
.\target\debug\all-in-one-desktop.exe
```

默认用户数据目录由 Tauri 的 `app_data_dir` 决定，Windows 通常是 `%APPDATA%\com.allinone.desktop`。多媒体功能额外需要 PATH 中的 `ffmpeg` / `ffprobe`；翻译需要配置对应服务；脚本需要相应解释器。缺少这些外部工具不应阻止主程序启动。

Windows 尚未接入系统凭据持久化和敏感剪贴板所有权。密码本的 Windows Hello 设备密钥只在 Passport 私钥解密成功后可用，PIN 不是 macOS Touch ID，硬件手势验收见 `docs/testing`。翻译密钥只能在当前会话内使用。
