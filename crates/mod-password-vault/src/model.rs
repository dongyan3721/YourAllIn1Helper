use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::Zeroizing;

/// 后端拥有的用户秘密；不实现 Debug/Clone，Drop 时清理自有字符串分配。
/// Serialize 仅用于显式 IPC 返回，序列化后的 JS/WebView 副本不在零化保证内。
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<&str> for SecretString {
    fn from(value: &str) -> Self {
        Self::new(value.to_string())
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl std::ops::Deref for SecretString {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl Serialize for SecretString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(Self::new)
    }
}

pub const MODULE_ID: &str = "password_vault";
pub const AUTO_LOCK_SECONDS: u64 = 180;
pub const REVEAL_SECONDS: u64 = 15;
pub const MAX_FIELD_BYTES: usize = 64 * 1024;
pub const MAX_ENTRY_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LockReason {
    Manual,
    Idle,
    ScreenLock,
    Sleep,
    PageClose,
    ModuleDisabled,
    IntegrityError,
    CapabilityError,
    PasswordChanged,
    Shutdown,
}

#[derive(Debug, Clone, Serialize)]
pub struct VaultEvent {
    pub epoch: u64,
    pub reason: LockReason,
}

#[derive(Debug, Clone, Serialize)]
pub struct VaultStatus {
    pub initialized: bool,
    pub locked: bool,
    pub unlocking: bool,
    pub epoch: u64,
    pub auto_lock_seconds: u64,
    pub biometric_available: bool,
    /// 当前库已经登记设备槽。没有槽时不能用 Touch ID 解锁。
    pub biometric_enrolled: bool,
    pub secret_copy_available: bool,
    pub capability_error: Option<String>,
}

/// 完整编辑输入不实现 Debug/Clone/Serialize，避免秘密被格式化或无意复制。
#[derive(Deserialize)]
pub struct VaultEntryInput {
    pub title: SecretString,
    pub url: SecretString,
    pub category: SecretString,
    pub username: SecretString,
    pub password: SecretString,
    pub totp_secret: SecretString,
    pub notes: SecretString,
    #[serde(default)]
    pub archived: bool,
}

/// IPC 序列化会产生前端明文副本；调用方必须按需短驻留并在锁定/卸载时清理。
#[derive(Serialize)]
pub struct VaultEntry {
    pub id: String,
    pub record_version: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived: bool,
    pub title: SecretString,
    pub url: SecretString,
    pub category: SecretString,
    pub username: SecretString,
    pub password: SecretString,
    pub totp_secret: SecretString,
    pub notes: SecretString,
}

#[derive(Serialize)]
pub struct VaultEntrySummary {
    pub id: String,
    pub record_version: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived: bool,
    pub title: SecretString,
    pub url: SecretString,
    pub category: SecretString,
    pub username: SecretString,
}

#[derive(Serialize)]
pub struct VaultEntryPage {
    pub entries: Vec<VaultEntrySummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VaultMutationResult {
    pub id: String,
    pub record_version: u64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PasswordPolicy {
    pub length: u16,
    pub uppercase: bool,
    pub lowercase: bool,
    pub digits: bool,
    pub symbols: bool,
}

impl Default for PasswordPolicy {
    fn default() -> Self {
        Self {
            length: 20,
            uppercase: true,
            lowercase: true,
            digits: true,
            symbols: true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum FieldId {
    Title = 1,
    Url = 2,
    Category = 3,
    Username = 4,
    Password = 5,
    TotpSecret = 6,
    Notes = 7,
}

impl FieldId {
    pub(crate) const ALL: [Self; 7] = [
        Self::Title,
        Self::Url,
        Self::Category,
        Self::Username,
        Self::Password,
        Self::TotpSecret,
        Self::Notes,
    ];

    pub(crate) fn from_u8(value: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|field| *field as u8 == value)
    }
}

impl VaultEntryInput {
    pub(crate) fn field(&self, field: FieldId) -> &str {
        match field {
            FieldId::Title => &self.title,
            FieldId::Url => &self.url,
            FieldId::Category => &self.category,
            FieldId::Username => &self.username,
            FieldId::Password => &self.password,
            FieldId::TotpSecret => &self.totp_secret,
            FieldId::Notes => &self.notes,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.title.trim().is_empty() {
            return Err("title is required");
        }
        let mut total = 0usize;
        for field in FieldId::ALL {
            let len = self.field(field).len();
            if len > MAX_FIELD_BYTES {
                return Err("field exceeds 64 KiB");
            }
            total = total.checked_add(len).ok_or("entry size overflow")?;
        }
        if total > MAX_ENTRY_BYTES {
            return Err("entry exceeds 256 KiB");
        }
        Ok(())
    }
}
