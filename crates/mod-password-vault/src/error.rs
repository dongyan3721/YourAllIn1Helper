use thiserror::Error;

/// 密码本错误不携带密码、明文、密钥或完整密文，避免日志与 IPC 意外泄露。
#[derive(Debug, Error)]
pub enum VaultError {
    #[error("vault is not initialized")]
    Uninitialized,
    #[error("vault is already initialized")]
    AlreadyInitialized,
    #[error("vault is locked")]
    Locked,
    #[error("another unlock is already in progress")]
    UnlockInProgress,
    #[error("master password unlock failed")]
    UnlockFailed,
    #[error("invalid vault input: {0}")]
    InvalidInput(&'static str),
    #[error("entry was not found")]
    NotFound,
    #[error("entry version conflict")]
    VersionConflict,
    #[error("operating system random source failed")]
    RandomSourceFailed,
    #[error("defensive nonce reuse check rejected the operation")]
    NonceReuse,
    #[error("vault data integrity check failed")]
    DataIntegrityError,
    #[error("unsupported or unsafe vault format")]
    UnsupportedFormat,
    #[error("module is disabled")]
    Disabled,
    #[error("feature is disabled in M1: {0}")]
    FeatureDisabled(&'static str),
    #[error("vault background task failed")]
    TaskJoin,
    #[error("database operation failed: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("core operation failed: {0}")]
    Core(#[from] app_core::CoreError),
}

pub type VaultResult<T> = Result<T, VaultError>;
