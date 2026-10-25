# 功能积木接入约定

本次改造沿用 PROJECT_PLAN.md 的微内核和 OS HAL 分层。十个功能槽位统一列在 `crates/app-core/src/catalog.rs`；当前实际挂载 01–09 共九个模块（01、07 为 M1，05 为首轮统计与报表），10 密码本仅完成 M0 设计审查并显示「待开发」，不创建空壳模块冒充已实现能力。

## 职责与目录

| 位置 | 职责 |
| --- | --- |
| `crates/app-core/src/module.rs` | AppModule 契约、活动状态、启停顺序和执行中的任务隔离 |
| `crates/app-core/src/config/schema.rs` | 配置结构与缺省值 |
| `crates/app-core/src/config/persistence.rs` | 文件读写、原子保存、v1 → v2 → v3 迁移 |
| `crates/os-hal/src/traits.rs` | 业务可见的系统能力契约 |
| `crates/os-hal/src/platform/mod.rs` | 原生适配器装配入口，不再堆叠功能实现 |
| `crates/os-hal/src/platform/{screen,input,regions,services,selection,usage}.rs` | 按能力组织的原生实现 |
| `crates/os-hal/src/platform/windows/` | 外部窗口能力和各操作系统实现 |
| `crates/os-hal/src/mock.rs` | 显式注入的测试适配器；生产适配器不再回退到 Mock |
| `crates/mod-*/src/` | 功能自身的领域逻辑 |
| `src-tauri/src/modules.rs` | 主程序唯一的具体功能装配点；命名字段返回同一组模块 Arc |
| `src-tauri/src/ipc/` | 按功能分离的 IPC 适配及统一开关检查 |
| `src-tauri/src/screenshot_binary.rs` | 大数据二进制 IPC：冻结帧/贴图以 `tauri::ipc::Response` 原始字节返回，标注层以原始请求体上传；同样经 `with_enabled` 检查 |
| `*/tests/unit/` | 从业务源码搬出的单元测试，保留访问私有实现的能力 |
| `*/tests/*.rs` | 对公共接口的集成与生命周期测试 |
| `src-tauri/tests/support/` | 显式运行的无桌面及桌面烟测支持代码 |
| `templates/module/` | 可独立编译、测试的功能模板 |

## 开关语义

在主控台「总览 → 功能积木」即时开关。`kernel_list_modules` 返回十个槽位的 `available`（是否挂载）、`enabled`（是否运行）、`supported`（系统能力）及原因；`kernel_set_module_enabled` 接收 `{"module_id":"clipboard","enabled":false}`。

- 启动时读取配置；关闭的功能不调用 on_init、不注册快捷键、不恢复窗口。
- 运行中关闭会等待已经进入模块的 IPC/后台操作完成，拒绝新操作，再调用 on_shutdown、保存配置、移除快捷键。Tauri 同步更新全局快捷键与托盘菜单。
- 剪贴板关闭后释放监听器使用的剪贴板对象，轮询协调线程休眠而不读取剪贴板。便笺关闭时销毁窗口并保留内容和打开标记，重新启用后恢复。窗口置顶关闭时还原已跟踪的窗口并清理拾取层。
- 停用不删除数据库表或用户记录；所有已挂载模块的迁移仍在启动时执行，以便后续启用和升级。
- 启用失败会清理本次初始化和快捷键；配置保存失败不发布新的内存配置。原生快捷键注册失败时，桌面 IPC 尝试还原之前的模块状态并返回错误。
- `on_init` 必须支持再次启用；`on_shutdown` 同时用于停用和退出，应可重复调用。后台任务必须可取消，持有的资源必须释放。生命周期钩子不要递归调用模块开关。
- 停用失败通过专门的 on_shutdown_failed 钩子恢复；恢复失败保持禁用并报告错误，不直接重复初始化。应用退出在后台排空模块，保持原生事件循环可处理窗口销毁。
- 后台任务经 `registry.with_enabled(id, || ...)` 进入业务操作；不要在闭包内部再嵌套获取同一注册表的活动锁。

运行态开关存在实际应用数据目录下的 `config.json`，仓库的 `config/config.example.json` 仅为配置示例。修改磁盘文件在下次启动读取；界面开关即时生效。旧的 `clipboard.enabled` 保留为「自动捕获」设置，与整个剪贴板积木的总开关独立。

```json
{
  "config_version": 3,
  "modules": {
    "enabled": { "clipboard": true, "sticky_note": false },
    "settings": { "example": { "show_window_title": true } }
  }
}
```

缺省开关为 true，但未挂载的功能始终不可调用。功能自己的配置结构放在自身 crate 的 `config.rs`，用 `ctx.module_settings::<MyConfig>("module_id")` 从 `modules.settings` 读取，避免核心层依赖业务配置类型。已有 typed legacy 设置保留，兼容原配置和调用端。主程序通过 `kernel.update_config` 原子保存设置，通过 `kernel.set_module_enabled` 改变开关，不直接修改 `modules.enabled`。

## 增加一个功能

1. 将 `templates/module` 复制为 `crates/mod-<name>`；调整 Cargo 包名和依赖相对路径，删除模板的 `[workspace]`，加入根 workspace members 和主程序依赖。
2. 改为目录里对应的稳定 module ID；只注入所需 Trait（例如 `Arc<dyn ScreenCapturer>`）。不要导入 `NativeOsHal`、`platform`、objc2 或 Win32。
3. 实现生命周期、迁移、快捷键；后台任务句柄归模块持有。配置类型和测试放各自文件。
4. 在 `src-tauri/src/modules.rs` 构造实例并通过 `register_module_arc` 挂载。同一个 Arc 用于注册表和 IPC，避免运行两个实例。
5. 在 `src-tauri/src/ipc/` 加适配器，并在路由中经 `with_enabled` 进入；前端根据目录状态显示入口。
6. 为启停、资源清理、禁用时拒绝调用、设置持久化写测试。目录中未挂载的槽位接入后自动变为 available。

## 平台边界

macOS 对外部原窗口持续置顶不受当前实现支持：适配器明确返回 Unsupported，主控台禁用置顶操作并保留窗口枚举和激活。移除了私有 CGS 伪成功、AXRaise 抢焦点兜底，以及修改任意用户窗口的旧测试。OCR、生物认证仍返回 Unsupported。划词读取已接入 macOS AX（需要无障碍权限），Windows UIAutomation 尚未实现。使用统计接入真实前台应用、空闲和会话状态；会话查询失败时暂停计费并报告原因，不按未锁屏继续累计。CLI 可能无法取得 CGSession，须结合实际 GUI 宿主测试。音频 AFK 例外尚未实现。

## 验证

```sh
cargo test --workspace --offline --all-targets
pnpm --dir src-ui run build
CARGO_TARGET_DIR=target cargo test --manifest-path templates/module/Cargo.toml --offline
cargo run -p all-in-one-host -- --ui-smoke-test
```

普通测试不需要桌面权限；原生窗口枚举测试显式 ignored。桌面烟测使用临时数据目录和本程序创建的 WebView，验证启停、销毁、恢复及快捷键同步，不写系统剪贴板、不模拟按键、不修改用户其他应用的窗口。


## 必备图形界面回归

每次涉及模块开关、窗口生命周期、输入或主题的修改，必须执行 [GUI 测试计划](testing/GUI_TEST_PLAN.md)，并提交对应日期的执行报告、构建 SHA-256 及截图证据。原生烟测或 IPC 测试不能替代实际点击、输入、窗口切换与重启验证。系统全局快捷键如不能通过自动化按键验证，应保留实体键盘复核项，不得标为通过。

人工 GUI 回归可通过 `ALL_IN_ONE_GUI_TEST_DATA_DIR` 隔离配置和数据库，继续使用真实 OS HAL；`scripts/prepare_gui_test_app.py` 可生成同一 release 可执行文件的隔离 QA App。
