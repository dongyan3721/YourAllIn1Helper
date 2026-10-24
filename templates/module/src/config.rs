//! 功能自身维护配置结构与缺省值，核心层只持久化 JSON。
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExampleConfig {
    pub show_window_title: bool,
}
