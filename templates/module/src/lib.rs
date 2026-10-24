//! 功能积木模板：仅通过构造函数注入所需系统能力。
pub mod config;
use app_core::{AppModule, CoreResult, ModuleContext, ShortcutSpec};
use os_hal::WindowController;
use std::sync::Arc;

pub struct ExampleModule {
    windows: Arc<dyn WindowController>,
}
impl ExampleModule {
    pub fn new(windows: Arc<dyn WindowController>) -> Self {
        Self { windows }
    }
}
impl AppModule for ExampleModule {
    fn id(&self) -> &'static str {
        "example"
    }
    fn name(&self) -> &'static str {
        "示例积木"
    }
    fn default_shortcuts(&self) -> Vec<ShortcutSpec> {
        vec![ShortcutSpec::new(
            self.id(),
            "example:run",
            "PrimaryMod+Shift+E",
            "运行示例",
        )]
    }
    fn on_init(&self, ctx: &ModuleContext) -> CoreResult<()> {
        // 在此初始化自有资源，关闭状态的积木不会进入该钩子。
        let _: config::ExampleConfig = ctx.module_settings(self.id())?;
        Ok(())
    }
    fn on_shortcut(&self, action: &str, ctx: &ModuleContext) -> CoreResult<()> {
        if action == "example:run" {
            let settings: config::ExampleConfig = ctx.module_settings(self.id())?;
            if settings.show_window_title {
                let _window = self
                    .windows
                    .get_foreground_window()
                    .map_err(|err| app_core::CoreError::Config(err.to_string()))?;
            }
        }
        Ok(())
    }
    fn on_shutdown(&self, _ctx: &ModuleContext) -> CoreResult<()> {
        // 取消并等待自有后台任务、关闭自有窗口、刷写状态，保留用户记录。
        // 关闭功能和退出程序都会调用该钩子，需要支持重复调用。
        Ok(())
    }
}
