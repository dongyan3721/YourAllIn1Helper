use app_core::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_concurrency() -> usize {
    2
}
fn default_history_limit() -> usize {
    200
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaProcConfig {
    pub output_dir: Option<PathBuf>,
    #[serde(default = "default_concurrency")]
    pub max_concurrent_jobs: usize,
    #[serde(default = "default_history_limit")]
    pub history_limit: usize,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for MediaProcConfig {
    fn default() -> Self {
        Self {
            output_dir: None,
            max_concurrent_jobs: default_concurrency(),
            history_limit: default_history_limit(),
            extra: serde_json::Map::new(),
        }
    }
}

impl MediaProcConfig {
    pub fn validate(&self) -> CoreResult<()> {
        if !(1..=4).contains(&self.max_concurrent_jobs) {
            return Err(CoreError::Config(
                "max_concurrent_jobs must be between 1 and 4".into(),
            ));
        }
        if !(10..=2_000).contains(&self.history_limit) {
            return Err(CoreError::Config(
                "history_limit must be between 10 and 2000".into(),
            ));
        }
        if let Some(path) = &self.output_dir {
            if path.as_os_str().is_empty() {
                return Err(CoreError::Config("output_dir cannot be empty".into()));
            }
        }
        Ok(())
    }
}
