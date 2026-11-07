//! 设备密钥信封与撤销顺序。不调用 Windows Hello，也不读取真实秘密。
use crate::{
    error::VaultError,
    hello_store::{parse_envelope, HelloBackend, WindowsHelloKeyStore, ENVELOPE_LEN, OAEP_LEN},
    keystore::{VaultKeyStore, WINDOWS_HELLO_BACKEND},
    model::DeviceKeyKind,
};
use std::{
    collections::HashMap,
    fs,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use tempfile::TempDir;
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Scripted {
    #[default]
    Ok,
    NotEnrolled,
    Canceled,
}

#[derive(Default)]
struct FakeHello {
    gate: Mutex<Scripted>,
    blobs: Mutex<HashMap<Uuid, Vec<u8>>>,
    unprotect: Mutex<Scripted>,
    unprotect_calls: AtomicUsize,
    delete_calls: AtomicUsize,
    bad_length: bool,
}

impl FakeHello {
    fn new() -> Self {
        Self::default()
    }
}

impl HelloBackend for FakeHello {
    fn gate(&self) -> crate::VaultResult<()> {
        match *self.gate.lock().expect("gate") {
            Scripted::NotEnrolled => Err(VaultError::BiometricNotEnrolled),
            Scripted::Canceled => Err(VaultError::BiometricCanceled),
            Scripted::Ok => Ok(()),
        }
    }

    fn protect_key(&self, slot_id: &Uuid, dkek: &[u8; 32]) -> crate::VaultResult<Vec<u8>> {
        if self.bad_length {
            return Ok(vec![1, 2, 3, 4]);
        }
        let mut wrapped = vec![0xA5; OAEP_LEN];
        wrapped[..32].copy_from_slice(dkek);
        for (index, byte) in wrapped.iter_mut().enumerate().take(16) {
            *byte ^= index as u8;
        }
        self.blobs
            .lock()
            .expect("blobs")
            .insert(*slot_id, wrapped.clone());
        Ok(wrapped)
    }

    fn unprotect_key(
        &self,
        slot_id: &Uuid,
        wrapped: &[u8],
    ) -> crate::VaultResult<Zeroizing<[u8; 32]>> {
        self.unprotect_calls.fetch_add(1, Ordering::SeqCst);
        match *self.unprotect.lock().expect("unprotect") {
            Scripted::Canceled => return Err(VaultError::BiometricCanceled),
            Scripted::NotEnrolled => return Err(VaultError::BiometricNotEnrolled),
            Scripted::Ok => {}
        }
        let stored = self
            .blobs
            .lock()
            .expect("blobs")
            .get(slot_id)
            .cloned()
            .ok_or(VaultError::BiometricKeyInvalid)?;
        if stored.as_slice() != wrapped {
            return Err(VaultError::BiometricKeyInvalid);
        }
        let mut dkek = Zeroizing::new([0u8; 32]);
        dkek.copy_from_slice(&stored[..32]);
        for (index, byte) in dkek.iter_mut().enumerate().take(16) {
            *byte ^= index as u8;
        }
        Ok(dkek)
    }

    fn delete_key(&self, slot_id: &Uuid) -> crate::VaultResult<()> {
        self.delete_calls.fetch_add(1, Ordering::SeqCst);
        self.blobs.lock().expect("blobs").remove(slot_id);
        Ok(())
    }
}

fn store(fake: Arc<FakeHello>, root: &std::path::Path) -> WindowsHelloKeyStore {
    WindowsHelloKeyStore::new(fake, root.to_path_buf())
}

#[test]
fn envelope_rejects_raw_dkek_and_wrong_header() {
    assert!(parse_envelope(&[7u8; 32]).is_err());
    assert!(parse_envelope(&[0u8; ENVELOPE_LEN]).is_err());
    let mut bytes = vec![0u8; ENVELOPE_LEN];
    bytes[..8].copy_from_slice(b"AIOHELLO");
    bytes[8] = 1;
    assert!(parse_envelope(&bytes).is_ok());
    bytes[8] = 2;
    assert!(matches!(
        parse_envelope(&bytes),
        Err(VaultError::BiometricKeyInvalid)
    ));
}

#[test]
fn protect_unprotect_revoke_and_repeat_do_not_keep_temp_files() {
    let temp = TempDir::new().expect("temp");
    let fake = Arc::new(FakeHello::new());
    let keystore = store(Arc::clone(&fake), temp.path());
    assert_eq!(keystore.backend_id(), WINDOWS_HELLO_BACKEND);
    assert_eq!(
        keystore.device_key_kind(),
        DeviceKeyKind::WindowsHelloGesture
    );
    let slot = Uuid::new_v4();
    let dkek = [9u8; 32];
    keystore.protect(&slot, &dkek).expect("protect");
    let path = temp.path().join(format!("{slot}.oaep"));
    assert!(path.is_file());
    assert!(!temp.path().join(format!("{slot}.tmp")).exists());
    let opened = keystore.unprotect(&slot).expect("unprotect");
    assert_eq!(opened.as_slice(), &dkek);
    let replacement = [4u8; 32];
    keystore.protect(&slot, &replacement).expect("repeat");
    assert_eq!(
        keystore.unprotect(&slot).expect("second").as_slice(),
        &replacement
    );
    keystore.revoke(&slot).expect("revoke");
    keystore.revoke(&slot).expect("revoke again");
    assert!(matches!(
        keystore.unprotect(&slot),
        Err(VaultError::BiometricKeyInvalid)
    ));
    assert!(!path.exists());
    assert!(!temp.path().join(format!("{slot}.tmp")).exists());
}

#[test]
fn cancel_and_unavailable_do_not_release_dkek() {
    let temp = TempDir::new().expect("temp");
    let fake = Arc::new(FakeHello::new());
    let keystore = store(Arc::clone(&fake), temp.path());
    let slot = Uuid::new_v4();
    keystore.protect(&slot, &[1u8; 32]).expect("protect");
    *fake.unprotect.lock().expect("error") = Scripted::Canceled;
    assert!(matches!(
        keystore.unprotect(&slot),
        Err(VaultError::BiometricCanceled)
    ));
    *fake.gate.lock().expect("gate") = Scripted::NotEnrolled;
    let other = Uuid::new_v4();
    assert!(matches!(
        keystore.protect(&other, &[2u8; 32]),
        Err(VaultError::BiometricNotEnrolled)
    ));
    assert!(!temp.path().join(format!("{other}.oaep")).exists());
}

#[test]
fn corrupt_envelope_does_not_call_unprotect_and_failed_rename_removes_temp() {
    let temp = TempDir::new().expect("temp");
    let fake = Arc::new(FakeHello::new());
    let keystore = store(Arc::clone(&fake), temp.path());
    let slot = Uuid::new_v4();
    let path = temp.path().join(format!("{slot}.oaep"));
    fs::write(&path, b"not-an-envelope").expect("corrupt");
    assert!(matches!(
        keystore.unprotect(&slot),
        Err(VaultError::BiometricKeyInvalid)
    ));
    assert_eq!(fake.unprotect_calls.load(Ordering::SeqCst), 0);

    fs::remove_file(&path).expect("remove corrupt file");
    fs::create_dir_all(&path).expect("directory blocks rename");
    let blocked = Arc::new(FakeHello::new());
    let blocked_store = store(Arc::clone(&blocked), temp.path());
    let result = blocked_store.protect(&slot, &[3u8; 32]);
    assert!(result.is_err());
    assert!(!temp.path().join(format!("{slot}.tmp")).exists());
    assert!(blocked.delete_calls.load(Ordering::SeqCst) >= 1);
}

#[test]
fn short_ciphertext_is_rejected_and_key_is_revoked() {
    let temp = TempDir::new().expect("temp");
    let fake = Arc::new(FakeHello {
        bad_length: true,
        ..FakeHello::new()
    });
    let keystore = store(Arc::clone(&fake), temp.path());
    let slot = Uuid::new_v4();
    assert!(keystore.protect(&slot, &[5u8; 32]).is_err());
    assert!(!temp.path().join(format!("{slot}.oaep")).exists());
    assert!(!temp.path().join(format!("{slot}.tmp")).exists());
    assert!(fake.delete_calls.load(Ordering::SeqCst) >= 1);
}
