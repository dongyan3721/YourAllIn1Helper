//! 贴图（`pin-img-{id}`）记录：只在窗口存在期间保留 PNG，关闭即释放。
use crate::geometry::LogicalRect;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

/// 贴图窗口标签前缀。
pub const PIN_LABEL_PREFIX: &str = "pin-img-";

pub fn pin_label(id: &str) -> String {
    format!("{}{}", PIN_LABEL_PREFIX, id)
}

/// 贴图 ID 只允许十六进制字符，避免窗口标签或路由注入。
pub fn is_valid_pin_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_hexdigit())
}

/// 贴图内存记录。
#[derive(Debug, Clone)]
pub struct PinRecord {
    pub id: String,
    pub png: Arc<Vec<u8>>,
    pub pixel_width: u32,
    pub pixel_height: u32,
    /// 100% 缩放时的逻辑尺寸（与截图时屏幕上看到的大小一致）。
    pub logical: LogicalRect,
    pub opacity: f64,
    pub created_at: i64,
}

/// 前端可见的贴图摘要（不含像素）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PinSummary {
    pub pin_id: String,
    pub label: String,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub logical_width: f64,
    pub logical_height: f64,
    pub opacity: f64,
    pub png_byte_length: usize,
    pub created_at: i64,
}

impl PinRecord {
    pub fn summary(&self) -> PinSummary {
        PinSummary {
            pin_id: self.id.clone(),
            label: pin_label(&self.id),
            pixel_width: self.pixel_width,
            pixel_height: self.pixel_height,
            logical_width: self.logical.width,
            logical_height: self.logical.height,
            opacity: self.opacity,
            png_byte_length: self.png.len(),
            created_at: self.created_at,
        }
    }
}

/// 按 ID 有序保存的贴图表。
#[derive(Default)]
pub struct PinStore {
    pins: RwLock<BTreeMap<String, PinRecord>>,
}

impl PinStore {
    pub fn insert(&self, record: PinRecord) {
        self.pins.write().insert(record.id.clone(), record);
    }

    pub fn get(&self, id: &str) -> Option<PinRecord> {
        self.pins.read().get(id).cloned()
    }

    pub fn remove(&self, id: &str) -> Option<PinRecord> {
        self.pins.write().remove(id)
    }

    pub fn ids(&self) -> Vec<String> {
        self.pins.read().keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.pins.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.pins.read().is_empty()
    }

    pub fn summaries(&self) -> Vec<PinSummary> {
        let mut list: Vec<PinSummary> = self.pins.read().values().map(PinRecord::summary).collect();
        list.sort_by_key(|p| p.created_at);
        list
    }
}
