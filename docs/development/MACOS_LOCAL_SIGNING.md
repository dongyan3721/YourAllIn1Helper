# macOS 本机固定开发签名

屏幕录制等 TCC 权限绑定 App 的代码签名要求。仅保持 Bundle ID 或文件名相同并不足够；临时签名的默认要求为 cdhash，每次编译可能变化。本项目打包入口现要求固定证书，不再静默临时签名。

## 本机配置
经过电脑所有者同意后，执行一次：

```sh
python3 scripts/macos_signing.py setup-local
python3 scripts/macos_signing.py check
bash scripts/package_macos_app.sh
```

setup-local 在登录钥匙串创建本机开发签名身份；私钥导入为不可导出，并只授权 /usr/bin/codesign 使用。仅添加当前用户域的 codeSign 信任，不添加系统域或 SSL 信任。中间私钥与 P12 存放在受保护临时目录，导入后清理；.runtime_data/macos-signing 仅保留公开证书与指纹配置，已被 Git 忽略。重复执行保留现有证书，不能每次构建重新生成。

已有 Apple Development / Developer ID 证书时可通过 MACOS_SIGNING_IDENTITY 指定钥匙串内的名称或指纹。本机自签名证书用于本机开发，不能冒充 Apple 签名/公证或面向其他电脑的正式发布。

## 迁移一次旧授权
1. 完全退出 All-in-One Toolbox，保持新构建文件不再更换。
2. 在系统设置的「隐私与安全 → 录屏与系统录音」选中本项目旧条目，移除（需要本人身份验证）。
3. 添加当前 dist/All-in-One Toolbox.app 并允许，再打开该 App。若系统要求，完全退出后再打开。
4. 检查截图页显示「已授权」，开始截图验证，Esc 取消；退出并重启复核。

不要重置所有应用的 TCC，不关闭 SIP/Gatekeeper，不以 identifier-only 的自定义签名要求绕过证书校验，不将窗口标题或图片像素当作权限证据。

## 后续构建
- 始终从 scripts/package_macos_app.sh 交付；原始 target 可执行文件和 Tauri 中间 bundle 不是最终固定签名产物。
- 最终 App 完整构建后签名一次，再执行 codesign --verify --strict；签名/验证失败保留旧交付包。
- dist/macos-build.json 记录公开签名要求、可执行文件 SHA-256、源码提交与脏工作区标志。
- 更新后可用 codesign -d -r- 查看要求：内容应包含证书指纹且保持不变，而非只有 cdhash。换证书需重新授权。
- 钥匙串证书/私钥不入库、不上传、不复制到其他人的机器。删除证书会使本机后续签名不可用，恢复时需新证书并迁移权限。

依据：[Apple DTS 对临时签名和屏幕权限的确认](https://developer.apple.com/forums/thread/819406)、[TN3127 代码签名要求](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements)。
