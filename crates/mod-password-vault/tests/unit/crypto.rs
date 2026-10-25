use crate::{
    crypto::{
        decrypt_field, derive_kek, encrypt_field, plaintext_to_string, unwrap_dek, wrap_dek,
        KdfProfile,
    },
    model::FieldId,
    VaultError,
};
use uuid::Uuid;

#[test]
fn field_envelopes_use_independent_nonces_and_authenticate_context() {
    let dek = [7u8; 32];
    let vault = Uuid::from_bytes([1; 16]);
    let entry = Uuid::from_bytes([2; 16]);
    let other_entry = Uuid::from_bytes([3; 16]);
    let plaintext = b"fictional-password-only";
    let first = encrypt_field(
        &dek,
        &vault,
        &entry,
        FieldId::Password,
        1,
        plaintext,
        &[4; 12],
    )
    .expect("first encryption");
    let second = encrypt_field(
        &dek,
        &vault,
        &entry,
        FieldId::Password,
        1,
        plaintext,
        &[5; 12],
    )
    .expect("second encryption");
    assert_ne!(first.0, second.0);
    assert_eq!(
        plaintext_to_string(
            decrypt_field(
                &dek,
                &vault,
                &entry,
                FieldId::Password,
                1,
                1,
                &[4; 12],
                &first.0,
                &first.1,
            )
            .expect("decrypt first")
        )
        .expect("utf8"),
        "fictional-password-only"
    );

    for (wrong_field, wrong_entry, wrong_version) in [
        (FieldId::Notes, entry, 1),
        (FieldId::Password, other_entry, 1),
        (FieldId::Password, entry, 2),
    ] {
        assert!(matches!(
            decrypt_field(
                &dek,
                &vault,
                &wrong_entry,
                wrong_field,
                1,
                wrong_version,
                &[4; 12],
                &first.0,
                &first.1,
            ),
            Err(VaultError::DataIntegrityError)
        ));
    }
}

#[test]
fn ciphertext_tag_and_nonce_tampering_release_no_plaintext() {
    let dek = [9u8; 32];
    let vault = Uuid::from_bytes([1; 16]);
    let entry = Uuid::from_bytes([2; 16]);
    let (ciphertext, tag) = encrypt_field(
        &dek,
        &vault,
        &entry,
        FieldId::Username,
        1,
        b"qa@example.test",
        &[8; 12],
    )
    .expect("encrypt");
    for target in 0..3 {
        let mut ciphertext = ciphertext.clone();
        let mut tag = tag;
        let mut nonce = [8; 12];
        match target {
            0 => ciphertext[0] ^= 1,
            1 => tag[0] ^= 1,
            _ => nonce[0] ^= 1,
        }
        assert!(matches!(
            decrypt_field(
                &dek,
                &vault,
                &entry,
                FieldId::Username,
                1,
                1,
                &nonce,
                &ciphertext,
                &tag,
            ),
            Err(VaultError::DataIntegrityError)
        ));
    }
}

#[test]
fn master_wrap_is_bound_to_vault_and_key() {
    let vault = Uuid::from_bytes([11; 16]);
    let other_vault = Uuid::from_bytes([12; 16]);
    let kek = [21; 32];
    let dek = [22; 32];
    let wrapped = wrap_dek(&kek, &vault, &dek, &[23; 12]).expect("wrap");
    assert_eq!(
        *unwrap_dek(&kek, &vault, &[23; 12], &wrapped).expect("unwrap"),
        dek
    );
    assert!(matches!(
        unwrap_dek(&kek, &other_vault, &[23; 12], &wrapped),
        Err(VaultError::UnlockFailed)
    ));
    assert!(matches!(
        unwrap_dek(&[24; 32], &vault, &[23; 12], &wrapped),
        Err(VaultError::UnlockFailed)
    ));
}

#[test]
fn kdf_profile_rejects_unknown_or_unbounded_parameters() {
    for (name, memory, iterations, lanes) in [
        ("argon2id-v19-m65536-t3-p4", 65_535, 3, 4),
        ("argon2id-v19-m65536-t3-p4", 65_536, 4, 4),
        ("argon2id-v19-m65536-t3-p4", 65_536, 3, 8),
        ("argon2i-v19", 65_536, 3, 4),
        ("attacker-controlled", u32::MAX, u32::MAX, u32::MAX),
    ] {
        assert!(matches!(
            KdfProfile::from_metadata(name, memory, iterations, lanes),
            Err(VaultError::UnsupportedFormat)
        ));
    }
    let profile =
        KdfProfile::from_metadata(KDF_PROFILE_NAME, 65_536, 3, 4).expect("reviewed profile");
    assert!(derive_kek(profile, b"password", b"short").is_err());
}

const KDF_PROFILE_NAME: &str = "argon2id-v19-m65536-t3-p4";
