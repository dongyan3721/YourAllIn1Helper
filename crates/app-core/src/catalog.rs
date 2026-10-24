//! 项目说明书中的十个功能槽位；只有挂载具体实现后才能开启运行。
use serde::Serialize;
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ModuleDefinition {
    pub number: u8,
    pub id: &'static str,
    pub name: &'static str,
}
pub const MODULE_CATALOG: &[ModuleDefinition] = &[
    ModuleDefinition {
        number: 1,
        id: "screenshot",
        name: "增强截图与贴图",
    },
    ModuleDefinition {
        number: 2,
        id: "clipboard",
        name: "剪贴板管理",
    },
    ModuleDefinition {
        number: 3,
        id: "screen_ink",
        name: "桌面屏幕注释",
    },
    ModuleDefinition {
        number: 4,
        id: "sticky_note",
        name: "桌面置顶便笺",
    },
    ModuleDefinition {
        number: 5,
        id: "app_usage",
        name: "应用使用时长统计",
    },
    ModuleDefinition {
        number: 6,
        id: "script_runner",
        name: "自定义脚本",
    },
    ModuleDefinition {
        number: 7,
        id: "translator",
        name: "全局划词翻译",
    },
    ModuleDefinition {
        number: 8,
        id: "media_proc",
        name: "基础多媒体处理",
    },
    ModuleDefinition {
        number: 9,
        id: "window_pin",
        name: "窗口置顶",
    },
    ModuleDefinition {
        number: 10,
        id: "password_vault",
        name: "生物识别密码本",
    },
];
