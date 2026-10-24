//! 配置独立于业务实现和数据库，分别维护结构定义与持久化。
mod persistence;
mod schema;
pub use persistence::ConfigMigrationPipeline;
pub use schema::*;
