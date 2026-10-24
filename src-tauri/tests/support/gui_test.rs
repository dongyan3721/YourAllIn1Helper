//! 图形界面人工回归的运行配置；不向主程序注入模拟能力或测试数据。
use std::path::PathBuf;

/// 指定目录后仍使用真实 OS HAL，只将数据库及模块配置与日常数据隔离。
pub(super) fn data_dir() -> Option<PathBuf> {
    std::env::var_os("ALL_IN_ONE_GUI_TEST_DATA_DIR").map(PathBuf::from)
}
