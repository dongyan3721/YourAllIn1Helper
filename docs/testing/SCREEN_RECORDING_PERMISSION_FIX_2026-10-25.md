# 屏幕录制权限失配修复（2026-10-25）

## 结论与直接证据
故障来自 TCC 内旧授权与当前临时签名身份不匹配；不是系统禁止本程序截图，也不是必须反复重启才能碰巧生效。

只读查询本项目系统日志得到：

```text
2026-10-25 19:10:39  Failed to match existing code requirement for subject com.allinone.desktop and service kTCCServiceScreenCapture
旧要求：cdhash H"20aaf34ec109335339a6da9b837fc400a0a9de0e"
当前要求：cdhash H"14997c6d949d93770905eb8d355103c68f421ab6"
```

当前旧构建经 codesign -d -r- 验证为 Signature=adhoc、TeamIdentifier=not set，designated requirement 只有 cdhash。系统设置虽显示 App 开启，但该开关保存的代码要求并不匹配正在运行的文件。Apple DTS 也[确认临时签名会使系统把重新构建视为新 App](https://developer.apple.com/forums/thread/819406)。

## 改动
- 按用户选择建立本机专用自签名开发证书，私钥位于登录钥匙串且不可导出，只授权 codesign 使用；仅用户域 codeSign 信任，不改变 SSL 或系统根信任。公开指纹配置位于被忽略的 .runtime_data/macos-signing；私钥/临时 P12/密码不入库。
- 新增 scripts/macos_signing.py；同机重复 setup-local 保留原身份。打包前检查证书；完整 App 最终签名并验证，不再先对裸二进制临时签名再覆盖 App 身份。拒绝无证书、临时签名或 identifier-only 作为修复构建。
- scripts/package_macos_app.sh 按 tauri.conf.json 的 productName 定位唯一产物，避免 find/head 误选遗留的旧名字 App。签名校验通过后替换 dist，保留公开构建清单。
- 权限适配器拆至独立 screen_permission.rs。被动状态只调用 CGPreflightScreenCaptureAccess；用户点击截图才调用 CGRequestScreenCaptureAccess，随后重新预检。删除按窗口标题/图像方差猜测权限及永久成功缓存，不因读取其他应用标题而当作授权。
- 截图按钮在缺权限时仍可点击申请，避免“必须授权才能点击申请”的入口死路；错误提示指向旧条目迁移。
- 新增申请/预检分离、成功后撤销应拒绝下一次截图、签名不得降级及旧身份不可再生成的回归测试。

## 一次迁移与 GUI 实测
用户明确选择本机自签名，并授权仅更新本项目录屏条目。移除旧条目时，系统要求身份验证，由用户亲自完成。随后在系统设置重新添加当前 dist/All-in-One Toolbox.app；其他应用权限未改动。

| 场景 | 结果 | 证据 |
| --- | --- | --- |
| 固定签名构建、迁移旧授权前 | 如实显示未授权，不伪造成功 | 实际 AX 观察；无额外画面采集探测 |
| 迁移授权后重新打开 | 系统预检为已授权 | [截图 01](evidence/screen-permission-2026-10-25/01-certificate-build-authorized.png) |
| 点击开始截图 | 会话进行中（Built-in Retina Display），窗口由 1 到 2 | [截图 02](evidence/screen-permission-2026-10-25/02-capture-session-started.png) |
| 取消截图 | 会话空闲、无冻结帧，窗口恢复 1 | 实际 GUI 观察 |
| 完全退出、只读确认进程消失、再次启动 | 仍显示已授权，无需再次审批 | [截图 03](evidence/screen-permission-2026-10-25/03-restarted-still-authorized.png) |
| 重新运行正式打包入口后，以 Cmd+Q 完全退出，确认旧进程结束，再次打开 | 新进程 24329（旧进程 23359），仍显示已授权，无需重新审批 | [截图 04](evidence/screen-permission-2026-10-25/04-repackaged-still-authorized.png) |

截图启动/取消验证不等同完整标注、保存、OCR、滚动/GIF 的 GUI 回归，本轮没有把实际桌面冻结帧写入仓库，也未覆盖系统剪贴板。

## 签名连续性与自动检查
临时 App 副本更改 CFBundleVersion 并重新签名后，CDHash 从 8d8165bc5e132c08d34f688bd2b1d0c2e34f6450 变为 7c098d3ba374bcc7eedee38b4d2bf15153ffa29b，但二者签名要求相同：

```text
identifier "com.allinone.desktop" and certificate leaf = H"271023d7c47579d97f8e05720c765a4af682d336"
```

对改动副本执行 codesign --verify --strict -R "=原要求" 通过，证明不再绑定单个构建哈希。测试副本未启动、不新增系统授权，交付 App 未被此实验更改。

- cargo test -p os-hal -p mod-screenshot --offline --all-targets：74 通过、0 失败、6 ignored。初次沙箱内两项真实 Vision OCR 测试失败；在正常桌面权限下同一测试集合通过。ignored 为显式系统操作/桌面测试，未擅自执行写剪贴板/钥匙串用例。
- 前端严格 TypeScript + Vite 构建通过；前端单测 24 通过。
- 签名策略 unittest 3 通过；bash -n 与 git diff --check 通过。
- 真实证书签名、codesign --verify --strict、跨构建证书要求验证通过。

本轮修复基于 195072f 和工作区已有的四处未提交尝试，保留用户的按钮申请入口并替换不可靠权限猜测。构建时源码尚未提交，构建清单保留 source_dirty=true 的真实状态；本次提交归档权限修复，未推送远端。后续构建要求固定证书，说明见 [本机签名约定](../development/MACOS_LOCAL_SIGNING.md)。本机证书用于本机开发，不等同 Apple 签名或公证发行。


## 最终产物
- 可执行文件 SHA-256：17a3f0b6eee1215ac7b5fe3d3f72dbe9222f63c64ab8c550ec988cc453331499。
- [构建及新旧进程记录](evidence/screen-permission-2026-10-25/build.json)；[签名连续性](evidence/screen-permission-2026-10-25/signing-continuity.json)。
- 最终重新打包前后可执行文件哈希也发生变化，但完全退出后的新进程仍有录屏授权；固定证书方案已通过本机实测。
- [Rust 测试日志](evidence/screen-permission-2026-10-25/rust-tests.log)、[前端构建](evidence/screen-permission-2026-10-25/ui-build.log)、[前端测试](evidence/screen-permission-2026-10-25/ui-tests.log)、[签名测试](evidence/screen-permission-2026-10-25/signing-tests.log)、[最终打包](evidence/screen-permission-2026-10-25/package.log)。
- 本轮保留其他功能的实现与测试状态，未宣称全部功能重新验收；当前 App 停留截图页供用户复核。

## 后续独立问题
用户随后反馈点击截图未显示蒙层。该问题尚未修复，将另行排查；本报告中的“截图会话启动”仅表示后台持有冻结帧并创建窗口，不代表蒙层实际可见或完整截图流程通过。上一轮最终回复应以此验收边界为准。
