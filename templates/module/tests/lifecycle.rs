//! 模板使用 Mock 验证启停，不依赖原生平台实现。
use app_core::AppKernelBuilder;
use mod_example::ExampleModule;
use os_hal::MockOsHal;
use std::sync::Arc;
#[test]
fn example_is_independent_of_the_native_platform_adapter() {
    let dir = tempfile::tempdir().unwrap();
    let kernel = AppKernelBuilder::new(dir.path())
        .with_in_memory_db(true)
        .register_module(ExampleModule::new(Arc::new(MockOsHal::new())))
        .unwrap()
        .bootstrap()
        .unwrap();
    kernel.trigger_action("example:run").unwrap();
    kernel.set_module_enabled("example", false).unwrap();
    assert!(kernel.trigger_action("example:run").is_err());
    kernel.set_module_enabled("example", true).unwrap();
    kernel.trigger_action("example:run").unwrap();
}
