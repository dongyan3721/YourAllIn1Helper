//! RFC 4226 (HOTP) / RFC 6238 (TOTP) 一次性密码。
//!
//! 密码条目的「TOTP 密钥」字段接受两种写法：
//! - 纯 Base32 密钥：按 Google Authenticator 默认值处理（TOTP、SHA-1、6 位、30 秒）。
//! - `otpauth://totp/...` 或 `otpauth://hotp/...` 链接：读取 `secret`、`algorithm`、
//!   `digits`、`period`（TOTP）与 `counter`（HOTP）。
//!
//! 计数器型 HOTP 的当前计数保存在同一个加密字段的 `counter` 参数里，随字段一起加密。

use crate::model::SecretString;
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use zeroize::Zeroizing;

const MAX_KEY_BYTES: usize = 1024;
const DEFAULT_DIGITS: u32 = 6;
const DEFAULT_PERIOD: u64 = 30;
const URI_PREFIX: &str = "otpauth://";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OtpKind {
    Totp,
    Hotp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OtpAlgorithm {
    Sha1,
    Sha256,
    Sha512,
}

/// 解析后的一次性密码配置。`key` 在 Drop 时清零。
pub(crate) struct OtpConfig {
    pub kind: OtpKind,
    pub algorithm: OtpAlgorithm,
    pub digits: u32,
    pub period: u64,
    pub counter: u64,
    key: Zeroizing<Vec<u8>>,
    /// 只有 otpauth 链接才能就地改写计数器；纯 Base32 永远是 TOTP。
    from_uri: bool,
}

/// 返回给界面的验证码。TOTP 带剩余时间，HOTP 带当前计数。
#[derive(Serialize)]
pub struct OtpCode {
    pub kind: OtpKind,
    pub code: SecretString,
    pub digits: u32,
    /// 仅 TOTP。
    pub period_secs: Option<u64>,
    /// 仅 TOTP：当前验证码还剩多少毫秒失效。
    pub remaining_ms: Option<u64>,
    /// 仅 HOTP：生成该验证码所用的计数值。
    pub counter: Option<u64>,
}

fn base32_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a'),
        b'2'..=b'7' => Some(byte - b'2' + 26),
        _ => None,
    }
}

/// 宽松的 RFC 4648 Base32：忽略空白、连字符与尾部 `=`，不区分大小写。
pub(crate) fn decode_base32(input: &str) -> Result<Zeroizing<Vec<u8>>, &'static str> {
    let mut output = Zeroizing::new(Vec::with_capacity(input.len() * 5 / 8 + 1));
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for byte in input.bytes() {
        if byte.is_ascii_whitespace() || byte == b'-' || byte == b'=' {
            continue;
        }
        let value = base32_value(byte).ok_or("secret is not valid Base32")?;
        buffer = (buffer << 5) | u32::from(value);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
        if output.len() > MAX_KEY_BYTES {
            return Err("secret is too long");
        }
    }
    if output.is_empty() {
        return Err("secret is empty");
    }
    Ok(output)
}

fn percent_decode(input: &str) -> Result<String, &'static str> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let hex = bytes
                    .get(index + 1..index + 3)
                    .and_then(|pair| std::str::from_utf8(pair).ok())
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                    .ok_or("invalid percent escape")?;
                out.push(hex);
                index += 3;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            other => {
                out.push(other);
                index += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| "invalid percent escape")
}

/// 在 `otpauth://` 链接里取第一个匹配的查询参数（键不区分大小写）。
fn query_param(query: &str, name: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .and_then(|(_, value)| percent_decode(value).ok())
}

fn parse_number<T: std::str::FromStr>(value: Option<String>, default: T) -> Result<T, &'static str> {
    match value {
        None => Ok(default),
        Some(text) => text.trim().parse::<T>().map_err(|_| "invalid numeric parameter"),
    }
}

impl OtpConfig {
    pub(crate) fn parse(input: &str) -> Result<Self, &'static str> {
        let input = input.trim();
        if input.is_empty() {
            return Err("secret is empty");
        }
        let is_uri = input
            .get(..URI_PREFIX.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(URI_PREFIX));
        if !is_uri {
            return Ok(Self {
                kind: OtpKind::Totp,
                algorithm: OtpAlgorithm::Sha1,
                digits: DEFAULT_DIGITS,
                period: DEFAULT_PERIOD,
                counter: 0,
                key: decode_base32(input)?,
                from_uri: false,
            });
        }
        let rest = &input[URI_PREFIX.len()..];
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let query = query.split('#').next().unwrap_or("");
        let kind = match path
            .split('/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "totp" => OtpKind::Totp,
            "hotp" => OtpKind::Hotp,
            _ => return Err("unsupported otpauth type"),
        };
        let secret = Zeroizing::new(query_param(query, "secret").ok_or("secret is missing")?);
        let algorithm = match query_param(query, "algorithm")
            .map(|value| value.to_ascii_uppercase())
            .as_deref()
        {
            None | Some("SHA1") => OtpAlgorithm::Sha1,
            Some("SHA256") => OtpAlgorithm::Sha256,
            Some("SHA512") => OtpAlgorithm::Sha512,
            Some(_) => return Err("unsupported algorithm"),
        };
        let digits = parse_number(query_param(query, "digits"), DEFAULT_DIGITS)?;
        if !(6..=8).contains(&digits) {
            return Err("digits must be 6 to 8");
        }
        let period = parse_number(query_param(query, "period"), DEFAULT_PERIOD)?;
        if !(1..=3600).contains(&period) {
            return Err("period must be 1 to 3600 seconds");
        }
        let counter = parse_number(query_param(query, "counter"), 0u64)?;
        Ok(Self {
            kind,
            algorithm,
            digits,
            period,
            counter,
            key: decode_base32(&secret)?,
            from_uri: true,
        })
    }

    /// 以给定计数值计算验证码。
    fn code_at(&self, counter: u64) -> Result<SecretString, &'static str> {
        let message = counter.to_be_bytes();
        let digest = match self.algorithm {
            OtpAlgorithm::Sha1 => hmac_bytes::<Hmac<Sha1>>(&self.key, &message),
            OtpAlgorithm::Sha256 => hmac_bytes::<Hmac<Sha256>>(&self.key, &message),
            OtpAlgorithm::Sha512 => hmac_bytes::<Hmac<Sha512>>(&self.key, &message),
        }?;
        let offset = usize::from(digest[digest.len() - 1] & 0x0f);
        let truncated = digest
            .get(offset..offset + 4)
            .ok_or("invalid HMAC output")?;
        let binary = u32::from_be_bytes([
            truncated[0] & 0x7f,
            truncated[1],
            truncated[2],
            truncated[3],
        ]);
        let value = binary % 10u32.pow(self.digits);
        Ok(SecretString::new(format!(
            "{value:0width$}",
            width = self.digits as usize
        )))
    }

    /// `now_ms` 为 Unix 毫秒时间戳。HOTP 忽略时间，使用配置里的当前计数。
    pub(crate) fn generate(&self, now_ms: i64) -> Result<OtpCode, &'static str> {
        match self.kind {
            OtpKind::Totp => {
                let now_ms = u64::try_from(now_ms).map_err(|_| "system clock is before 1970")?;
                let period_ms = self.period * 1000;
                Ok(OtpCode {
                    kind: OtpKind::Totp,
                    code: self.code_at(now_ms / period_ms)?,
                    digits: self.digits,
                    period_secs: Some(self.period),
                    remaining_ms: Some(period_ms - now_ms % period_ms),
                    counter: None,
                })
            }
            OtpKind::Hotp => Ok(OtpCode {
                kind: OtpKind::Hotp,
                code: self.code_at(self.counter)?,
                digits: self.digits,
                period_secs: None,
                remaining_ms: None,
                counter: Some(self.counter),
            }),
        }
    }

    /// 计数器型 HOTP 才能推进；返回改写了 `counter` 的新链接字段值。
    pub(crate) fn advanced_secret(
        &self,
        original: &str,
    ) -> Result<(u64, SecretString), &'static str> {
        if self.kind != OtpKind::Hotp || !self.from_uri {
            return Err("secret is not counter based");
        }
        let next = self
            .counter
            .checked_add(1)
            .ok_or("counter overflow")?;
        Ok((next, set_counter_param(original.trim(), next)))
    }
}

fn hmac_bytes<M: Mac + hmac::digest::KeyInit>(
    key: &[u8],
    message: &[u8],
) -> Result<Zeroizing<Vec<u8>>, &'static str> {
    let mut mac = <M as Mac>::new_from_slice(key).map_err(|_| "invalid HMAC key")?;
    mac.update(message);
    Ok(Zeroizing::new(mac.finalize().into_bytes().to_vec()))
}

/// 替换（或追加）链接中的第一个 `counter` 参数，其余内容原样保留。
fn set_counter_param(uri: &str, counter: u64) -> SecretString {
    let (base, query) = uri.split_once('?').unwrap_or((uri, ""));
    let mut replaced = false;
    let mut parts: Vec<String> = query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let key = part.split('=').next().unwrap_or("");
            if !replaced && key.eq_ignore_ascii_case("counter") {
                replaced = true;
                format!("{key}={counter}")
            } else {
                part.to_string()
            }
        })
        .collect();
    if !replaced {
        parts.push(format!("counter={counter}"));
    }
    SecretString::new(format!("{base}?{}", parts.join("&")))
}
