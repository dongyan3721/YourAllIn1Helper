use super::support::{input, Fixture};
use crate::{
    totp::{decode_base32, OtpConfig},
    OtpKind, VaultError,
};

const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// 测试专用 Base32 编码，用来把 RFC 附录里的 ASCII 密钥变成用户会输入的形式。
fn base32(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut buffer, mut bits) = (0u32, 0u32);
    for byte in bytes {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn rfc_secret(len: usize) -> String {
    let seed = b"1234567890";
    let bytes: Vec<u8> = (0..len).map(|i| seed[i % seed.len()]).collect();
    base32(&bytes)
}

fn code_at(config: &OtpConfig, seconds: i64) -> String {
    config
        .generate(seconds * 1000)
        .expect("generate")
        .code
        .as_str()
        .to_string()
}

#[test]
fn base32_is_lenient_about_case_spacing_and_padding() {
    let canonical = decode_base32("GEZDGNBVGY3TQOJQ").expect("canonical");
    assert_eq!(canonical.as_slice(), b"1234567890");
    let sloppy = decode_base32(" gezd gnbv-gy3t qojq== ").expect("sloppy");
    assert_eq!(sloppy.as_slice(), b"1234567890");
    assert!(decode_base32("GEZD1").is_err());
    assert!(decode_base32("  ").is_err());
}

#[test]
fn hotp_matches_rfc_4226_vectors() {
    let expected = [
        "755224", "287082", "359152", "969429", "338314", "254676", "287922", "162583", "399871",
        "520489",
    ];
    for (counter, want) in expected.iter().enumerate() {
        let uri = format!(
            "otpauth://hotp/Test?secret={}&counter={counter}",
            rfc_secret(20)
        );
        let config = OtpConfig::parse(&uri).expect("parse hotp");
        assert_eq!(config.kind, OtpKind::Hotp);
        let code = config.generate(0).expect("generate");
        assert_eq!(code.code.as_str(), *want, "counter {counter}");
        assert_eq!(code.counter, Some(counter as u64));
        assert!(code.remaining_ms.is_none());
    }
}

#[test]
fn totp_matches_rfc_6238_vectors_for_all_algorithms() {
    let cases: [(&str, usize, [(i64, &str); 4]); 3] = [
        (
            "SHA1",
            20,
            [
                (59, "94287082"),
                (1111111109, "07081804"),
                (1234567890, "89005924"),
                (20000000000, "65353130"),
            ],
        ),
        (
            "SHA256",
            32,
            [
                (59, "46119246"),
                (1111111109, "68084774"),
                (1234567890, "91819424"),
                (20000000000, "77737706"),
            ],
        ),
        (
            "SHA512",
            64,
            [
                (59, "90693936"),
                (1111111109, "25091201"),
                (1234567890, "93441116"),
                (20000000000, "47863826"),
            ],
        ),
    ];
    for (algorithm, key_len, vectors) in cases {
        let uri = format!(
            "otpauth://totp/Test?secret={}&algorithm={algorithm}&digits=8&period=30",
            rfc_secret(key_len)
        );
        let config = OtpConfig::parse(&uri).expect("parse totp");
        for (seconds, want) in vectors {
            assert_eq!(code_at(&config, seconds), want, "{algorithm} at {seconds}");
        }
    }
}

#[test]
fn bare_secret_uses_google_authenticator_defaults_and_reports_remaining_time() {
    let config = OtpConfig::parse(&rfc_secret(20)).expect("bare secret");
    assert_eq!(config.kind, OtpKind::Totp);
    assert_eq!((config.digits, config.period), (6, 30));
    // RFC 向量 59 秒的 8 位码为 94287082，6 位即末六位。
    let code = config.generate(59_000).expect("generate");
    assert_eq!(code.code.as_str(), "287082");
    assert_eq!(code.period_secs, Some(30));
    assert_eq!(code.remaining_ms, Some(1_000));
    let boundary = config.generate(60_000).expect("boundary");
    assert_eq!(boundary.remaining_ms, Some(30_000));
    assert_ne!(boundary.code.as_str(), code.code.as_str());
}

#[test]
fn uri_parameters_are_validated() {
    let secret = rfc_secret(20);
    for bad in [
        format!("otpauth://totp/x?secret={secret}&digits=5"),
        format!("otpauth://totp/x?secret={secret}&digits=9"),
        format!("otpauth://totp/x?secret={secret}&period=0"),
        format!("otpauth://totp/x?secret={secret}&period=99999"),
        format!("otpauth://totp/x?secret={secret}&algorithm=MD5"),
        format!("otpauth://totp/x?secret={secret}&counter=abc"),
        "otpauth://totp/x?issuer=nosecret".to_string(),
        "otpauth://steam/x?secret=GEZDGNBVGY3TQOJQ".to_string(),
        "otpauth://totp/x?secret=!!!".to_string(),
        String::new(),
    ] {
        assert!(OtpConfig::parse(&bad).is_err(), "{bad}");
    }
    let escaped = OtpConfig::parse("OTPAUTH://TOTP/Acme%3Aalice?issuer=Acme&secret=GEZD%20GNBV%20GY3T%20QOJQ")
        .expect("case-insensitive scheme and percent escapes");
    assert_eq!(escaped.digits, 6);
}

#[test]
fn advancing_rewrites_only_the_counter_parameter() {
    let secret = rfc_secret(20);
    let uri = format!("otpauth://hotp/Acme:alice?secret={secret}&issuer=Acme&counter=7&digits=8");
    let config = OtpConfig::parse(&uri).expect("parse");
    let (next, rewritten) = config.advanced_secret(&uri).expect("advance");
    assert_eq!(next, 8);
    assert_eq!(
        rewritten.as_str(),
        format!("otpauth://hotp/Acme:alice?secret={secret}&issuer=Acme&counter=8&digits=8")
    );

    let no_counter = format!("otpauth://hotp/x?secret={secret}");
    let config = OtpConfig::parse(&no_counter).expect("parse");
    let (next, rewritten) = config.advanced_secret(&no_counter).expect("advance");
    assert_eq!(next, 1);
    assert_eq!(
        rewritten.as_str(),
        format!("otpauth://hotp/x?secret={secret}&counter=1")
    );

    let totp = OtpConfig::parse(&secret).expect("bare");
    assert!(totp.advanced_secret(&secret).is_err());
}

#[tokio::test]
async fn service_generates_totp_without_returning_the_secret() {
    let fixture = Fixture::unlocked().await;
    let ctx = fixture.kernel.context().clone();
    let mut entry = input("totp");
    entry.totp_secret = rfc_secret(20).into();
    let created = fixture
        .service
        .create_entry(ctx.clone(), entry)
        .await
        .expect("create");
    let code = fixture
        .service
        .otp_code(ctx.clone(), &created.id)
        .await
        .expect("code");
    assert_eq!(code.kind, OtpKind::Totp);
    assert_eq!(code.code.len(), 6);
    assert!(code.code.chars().all(|c| c.is_ascii_digit()));
    let remaining = code.remaining_ms.expect("remaining");
    assert!((1..=30_000).contains(&remaining));

    let page = fixture
        .service
        .list_entries(ctx.clone(), None, None, None)
        .await
        .expect("list");
    assert!(page.entries[0].has_totp);

    let mut blank = input("blank");
    blank.totp_secret = "   ".into();
    let blank = fixture
        .service
        .create_entry(ctx.clone(), blank)
        .await
        .expect("create blank");
    assert!(matches!(
        fixture.service.otp_code(ctx.clone(), &blank.id).await,
        Err(VaultError::OtpNotConfigured)
    ));

    let mut broken = input("broken");
    broken.totp_secret = "otpauth://totp/x?digits=6".into();
    let broken = fixture
        .service
        .create_entry(ctx.clone(), broken)
        .await
        .expect("create broken");
    assert!(matches!(
        fixture.service.otp_code(ctx.clone(), &broken.id).await,
        Err(VaultError::OtpInvalid(_))
    ));
    // 配置错误不能把整个密码本锁掉。
    assert!(!fixture.service.status().locked);
}

#[tokio::test]
async fn hotp_counter_is_persisted_encrypted_and_guarded_against_double_advance() {
    let fixture = Fixture::unlocked().await;
    let ctx = fixture.kernel.context().clone();
    let mut entry = input("hotp");
    entry.totp_secret =
        format!("otpauth://hotp/Test?secret={}&counter=0", rfc_secret(20)).into();
    let created = fixture
        .service
        .create_entry(ctx.clone(), entry)
        .await
        .expect("create");
    let before = fixture
        .service
        .get_entry(ctx.clone(), &created.id)
        .await
        .expect("before");

    let first = fixture
        .service
        .otp_code(ctx.clone(), &created.id)
        .await
        .expect("counter 0");
    assert_eq!(first.code.as_str(), "755224");
    assert_eq!(first.counter, Some(0));
    // 读取验证码不推进计数。
    let again = fixture
        .service
        .otp_code(ctx.clone(), &created.id)
        .await
        .expect("still counter 0");
    assert_eq!(again.code.as_str(), "755224");

    let advanced = fixture
        .service
        .otp_advance_counter(ctx.clone(), &created.id, 0)
        .await
        .expect("advance");
    assert_eq!(advanced.code.code.as_str(), "287082");
    assert_eq!(advanced.code.counter, Some(1));
    assert_eq!(advanced.record_version, created.record_version + 1);
    assert!(advanced.totp_secret.contains("counter=1"));

    // 同一个旧计数再次推进必须被拒绝，防止双击跳号。
    assert!(matches!(
        fixture
            .service
            .otp_advance_counter(ctx.clone(), &created.id, 0)
            .await,
        Err(VaultError::OtpCounterConflict)
    ));

    let after = fixture
        .service
        .get_entry(ctx.clone(), &created.id)
        .await
        .expect("after");
    assert_eq!(after.record_version, advanced.record_version);
    assert_eq!(after.updated_at, before.updated_at);
    assert_eq!(after.title.as_str(), before.title.as_str());
    assert_eq!(after.password.as_str(), before.password.as_str());
    assert_eq!(after.notes.as_str(), before.notes.as_str());
    assert_eq!(after.totp_secret.as_str(), advanced.totp_secret.as_str());

    // 重新锁定再打开后，计数仍然是加密保存的 1。
    let reopened = fixture.reopen();
    reopened.unlock().await;
    let persisted = reopened
        .service
        .otp_code(reopened.kernel.context().clone(), &created.id)
        .await
        .expect("persisted");
    assert_eq!(persisted.code.as_str(), "287082");

    // TOTP 型密钥不能被当作计数器推进。
    let mut plain = input("plain");
    plain.totp_secret = rfc_secret(20).into();
    let plain = fixture
        .service
        .create_entry(ctx.clone(), plain)
        .await
        .expect("create totp");
    assert!(matches!(
        fixture
            .service
            .otp_advance_counter(ctx, &plain.id, 0)
            .await,
        Err(VaultError::OtpInvalid(_))
    ));
}

#[tokio::test]
async fn otp_operations_require_an_unlocked_vault() {
    let fixture = Fixture::initialized().await;
    let ctx = fixture.kernel.context().clone();
    let id = uuid::Uuid::nil().to_string();
    assert!(matches!(
        fixture.service.otp_code(ctx.clone(), &id).await,
        Err(VaultError::Locked)
    ));
    assert!(matches!(
        fixture.service.otp_advance_counter(ctx, &id, 0).await,
        Err(VaultError::Locked)
    ));
}
