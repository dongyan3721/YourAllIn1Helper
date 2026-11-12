# Windows 发布维护

## 构建

需要 Windows x64、Rust MSVC 工具链、Visual Studio C++ Build Tools、Node.js 和 pnpm。

```powershell
./scripts/build_windows_release.ps1
```

脚本先安装锁定的前端依赖、检查 TypeScript，再构建优化版 MSI 和更新签名。产物位于 `target/release/bundle/msi`。

更新私钥保存在当前维护者机器的 `%LOCALAPPDATA%\YourAllIn1Helper\release-keys\updater.key`，不在仓库中。请单独备份该文件；不要提交、上传或更换现有安装版使用的密钥。可通过脚本的 `-SigningKey` 指定备份恢复后的路径。公开公钥内嵌在 `src-tauri/tauri.windows.conf.json`，并随 release 发布。MSI 与自动更新配置放在 Windows 平台配置中，通用配置保留其他平台的默认打包方式。

## 版本与安装身份

同步修改工作区 Cargo 版本、两个 package.json 和 tauri.conf.json 的应用版本。应用使用 SemVer（当前 `0.1.3-beta`）；MSI 使用三段数字版本（当前 `0.1.3`）。后续更新须递增 MSI 三段数字版本，例如 `0.1.2-beta` 配合 `0.1.2`；不要仅递增 beta 后缀后推送自动更新。

以下身份必须保持稳定：

- 应用 identifier：`com.allinone.desktop`。
- MSI UpgradeCode：`a3a2c967-c2d8-50a2-9db5-8a8a436126bb`。
- publisher：`dongyan3721`，产品名：`All-in-One Toolbox`。
- 每用户安装范围、HKCU 安装路径注册项和更新公钥。

自定义 WiX 模板基于 Tauri CLI 2.12.4 的官方模板，升级 CLI 时应对照上游模板核对。安装器只管理程序目录和快捷方式，不删除应用数据目录。

## 准备、推送与发布

```powershell
python scripts/publish_windows_release.py --prepare "target/release/bundle/msi/<实际文件名>.msi" --notes docs/releases/0.1.3-beta.md
```

这会生成 `dist/v0.1.3-beta` 下的 MSI、签名、校验和、公开公钥、说明和 `latest.json`，同时更新受版本控制的 `updates/windows-beta.json`。检查文件和签名、完成回归和升级兼容性验证后，将代码及 feed 提交在发布分支，推送该分支及 `v0.1.3-beta` 标签。先不要更新 master 上的 feed。

```powershell
python scripts/publish_windows_release.py --publish --finalize
```

可先运行 `--publish` 上传草稿附件并核对 GitHub 返回的 SHA256，再运行 `--publish --finalize` 公开 prerelease。确认附件可下载后，将发布提交快进合并并推送到 master，使自动更新 feed 指向已就绪的附件。

发布脚本通过现有 Git Credential Manager 或 `GITHUB_TOKEN` 认证，不输出凭据。标签必须指向当前提交。脚本不会替换已公开 release 的附件。更新 feed 使用 master 上的静态 JSON，因此支持 beta 发布；master 的 feed 只能在对应附件公开后更新。

## 自动升级行为

仅注册安装路径内的 release EXE 启用后台更新；debug、GUI QA 与便携副本不参与。启动 15 秒后首次检查，随后每 6 小时检查。Tauri updater 下载并验证签名，将已验证的更新保留到本次运行正常退出。

退出先完成现有便笺刷新和模块关闭事务，再调用 updater 启动每用户 MSI `/quiet /norestart REBOOT=ReallySuppress`。便笺未能保存时会取消退出，也不会安装更新。更新不强制重启应用；用户下次启动进入新版本。断网或未发现更新时保持原行为。

本版仅有 updater 签名；若后续购买 Authenticode 证书，应在构建配置中另外配置安装包代码签名。
