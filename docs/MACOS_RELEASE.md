# macOS beta 发布

首次生成独立 macOS updater 密钥：`src-ui/node_modules/.bin/tauri signer generate --ci -p '' -w "$HOME/Library/Application Support/YourAllIn1Helper/release-keys/macos-updater.key"`。私钥必须留在本机受保护目录且备份，不得进入 Git 或 Release；仅将 `.pub` 内容配置到 `tauri.macos.conf.json`。不得替换 Windows 的密钥或 feed。

执行 `scripts/build_macos_release.sh docs/releases/<version>.md`。先严格检查前端，再构建 App，以固定钥匙串身份签名并验证，再生成 DMG 和包含完整签名 App 的 updater tar.gz。脚本按宿主架构生成真实平台条目，不声明尚未构建的架构。私钥路径可用 `TAURI_SIGNING_PRIVATE_KEY_PATH` 指定，密码由 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 提供。

产物在 `dist/v<version>/`：DMG、tar.gz、tar.gz.sig、公钥、feed、构建身份记录、发布说明和 SHA256SUMS。重新执行签名更新包验证测试，检查原包通过、篡改包失败；挂载 DMG 核对 App 签名、架构和版本，并实际进行 GUI 回归。

创建新的预发布 tag，不覆盖既有 Windows Release。先上传和发布完整资产，再推送 master 上的 `updates/macos-beta.json`，避免在线 feed 引用尚不可下载的资产。构建记录中的 source_head/source_dirty 指明构建时源代码状态，后续 QA 文档或 feed 提交可能使发布 tag 晚于构建源提交。

自签名证书仅维持本机签名身份与权限连续性，不能代替 Apple Developer ID 与公证；发布说明必须明确这一限制。系统身份验证由用户本人完成。
