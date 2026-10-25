use super::support::{input, CountingRandom, Fixture, MASTER};
use crate::{store, LockReason, PasswordPolicy, VaultError};
use rusqlite::params;
use std::{sync::Arc, time::Duration};
use zeroize::Zeroizing;

#[tokio::test]
async fn initialize_persists_only_ciphertext_and_reopen_unlocks() {
    let fixture = Fixture::initialized().await;
    assert!(fixture.service.status().locked);
    fixture.unlock().await;
    let entry = fixture
        .service
        .create_entry(fixture.kernel.context().clone(), input("alpha"))
        .await
        .expect("create encrypted entry");
    let conn = fixture.kernel.context().db.lock();
    let (ciphertext, metadata_plaintext): (Vec<u8>, Option<String>) = conn
        .query_row(
            "SELECT f.ciphertext, NULL
             FROM password_vault_fields f
             JOIN password_vault_entries e ON e.id = f.entry_id
             WHERE e.id = ?1 AND f.field_id = 1",
            params![entry.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read fictional ciphertext");
    assert!(!ciphertext
        .windows("虚构站点".len())
        .any(|w| w == "虚构站点".as_bytes()));
    assert!(metadata_plaintext.is_none());
    assert!(conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'vault_device_slots')",
            [],
            |row| row.get::<_, bool>(0),
        )
        .expect("device table exists"));
    drop(conn);

    let reopened = fixture.reopen();
    assert!(reopened.service.status().locked);
    reopened.unlock().await;
    let page = reopened
        .service
        .list_entries(reopened.kernel.context().clone(), None, None, None)
        .await
        .expect("list after reopen");
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].title, "虚构站点 alpha");
}

#[tokio::test]
async fn wrong_password_and_every_sensitive_operation_fail_when_locked() {
    let fixture = Fixture::initialized().await;
    assert!(matches!(
        fixture
            .service
            .unlock_master_password(
                fixture.kernel.context().clone(),
                Zeroizing::new("definitely-wrong-fiction".to_string()),
            )
            .await,
        Err(VaultError::UnlockFailed)
    ));
    let ctx = fixture.kernel.context().clone();
    assert!(matches!(
        fixture
            .service
            .list_entries(ctx.clone(), None, None, None)
            .await,
        Err(VaultError::Locked)
    ));
    assert!(matches!(
        fixture
            .service
            .get_entry(ctx.clone(), &uuid::Uuid::nil().to_string())
            .await,
        Err(VaultError::Locked)
    ));
    assert!(matches!(
        fixture
            .service
            .reveal_password(ctx.clone(), &uuid::Uuid::nil().to_string())
            .await,
        Err(VaultError::Locked)
    ));
    assert!(matches!(
        fixture
            .service
            .create_entry(ctx.clone(), input("locked"))
            .await,
        Err(VaultError::Locked)
    ));
    assert!(matches!(
        fixture
            .service
            .update_entry(
                ctx.clone(),
                &uuid::Uuid::nil().to_string(),
                1,
                input("locked")
            )
            .await,
        Err(VaultError::Locked)
    ));
    assert!(matches!(
        fixture
            .service
            .delete_entry(ctx.clone(), &uuid::Uuid::nil().to_string(), 1)
            .await,
        Err(VaultError::Locked)
    ));
    assert!(matches!(
        fixture
            .service
            .change_master_password(
                ctx,
                Zeroizing::new(MASTER.to_string()),
                Zeroizing::new("another-fiction-master".to_string()),
            )
            .await,
        Err(VaultError::Locked)
    ));
    assert!(matches!(
        fixture.service.copy_password_disabled(),
        Err(VaultError::FeatureDisabled(_))
    ));
    assert!(matches!(
        fixture.service.biometric_disabled(),
        Err(VaultError::FeatureDisabled(_))
    ));
}

#[tokio::test]
async fn crud_search_reveal_and_password_generator_use_only_fictional_values() {
    let fixture = Fixture::unlocked().await;
    let ctx = fixture.kernel.context().clone();
    let first = fixture
        .service
        .create_entry(ctx.clone(), input("alpha"))
        .await
        .expect("first");
    fixture
        .service
        .create_entry(ctx.clone(), input("beta"))
        .await
        .expect("second");
    let page = fixture
        .service
        .list_entries(ctx.clone(), Some("自动化测试".to_string()), None, Some(1))
        .await
        .expect("search notes");
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.next_cursor.as_deref(), Some("1"));
    let second_page = fixture
        .service
        .list_entries(
            ctx.clone(),
            Some("自动化测试".to_string()),
            page.next_cursor,
            Some(1),
        )
        .await
        .expect("second page");
    assert_eq!(second_page.entries.len(), 1);
    assert!(second_page.next_cursor.is_none());

    let full = fixture
        .service
        .get_entry(ctx.clone(), &first.id)
        .await
        .expect("full entry");
    assert_eq!(full.password, "not-a-real-password-alpha");
    assert_eq!(full.totp_secret, "FAKE-TOTP-alpha");
    let revealed = fixture
        .service
        .reveal_password(ctx.clone(), &first.id)
        .await
        .expect("reveal");
    assert_eq!(&*revealed, "not-a-real-password-alpha");
    assert!(fixture
        .service
        .delete_entry(ctx.clone(), &first.id, first.record_version)
        .await
        .is_ok());
    assert!(matches!(
        fixture.service.get_entry(ctx.clone(), &first.id).await,
        Err(VaultError::NotFound)
    ));

    let generated = fixture
        .service
        .generate_password(PasswordPolicy::default())
        .expect("generate");
    assert_eq!(generated.len(), 20);
    assert!(generated.chars().any(|c| c.is_ascii_uppercase()));
    assert!(generated.chars().any(|c| c.is_ascii_lowercase()));
    assert!(generated.chars().any(|c| c.is_ascii_digit()));
    assert!(generated.chars().any(|c| "!@#$%^&*()-_=+[]{}?".contains(c)));
}

#[tokio::test]
async fn update_reencrypts_every_field_and_rejects_stale_version() {
    let fixture = Fixture::unlocked().await;
    let ctx = fixture.kernel.context().clone();
    let created = fixture
        .service
        .create_entry(ctx.clone(), input("before"))
        .await
        .expect("create");
    let nonces_before = field_nonces(&fixture, &created.id);
    let mut changed = input("after");
    changed.archived = true;
    let updated = fixture
        .service
        .update_entry(ctx.clone(), &created.id, 1, changed)
        .await
        .expect("update");
    assert_eq!(updated.record_version, 2);
    let nonces_after = field_nonces(&fixture, &created.id);
    assert_eq!(nonces_before.len(), 7);
    assert_eq!(nonces_after.len(), 7);
    for ((field_before, nonce_before, version_before), (field_after, nonce_after, version_after)) in
        nonces_before.into_iter().zip(nonces_after)
    {
        assert_eq!(field_before, field_after);
        assert_eq!(version_before, 1);
        assert_eq!(version_after, 2);
        assert_ne!(nonce_before, nonce_after);
    }
    assert!(matches!(
        fixture
            .service
            .update_entry(ctx.clone(), &created.id, 1, input("stale"))
            .await,
        Err(VaultError::VersionConflict)
    ));
}

fn field_nonces(fixture: &Fixture, entry_id: &str) -> Vec<(i64, Vec<u8>, i64)> {
    let conn = fixture.kernel.context().db.lock();
    let mut stmt = conn
        .prepare("SELECT field_id, nonce, record_version FROM password_vault_fields WHERE entry_id = ?1 ORDER BY field_id")
        .expect("prepare nonce query");
    stmt.query_map(params![entry_id], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })
    .expect("nonce query")
    .collect::<Result<Vec<_>, _>>()
    .expect("nonce rows")
}

#[tokio::test]
async fn ciphertext_tampering_locks_session_and_releases_no_secret() {
    let fixture = Fixture::unlocked().await;
    let ctx = fixture.kernel.context().clone();
    let created = fixture
        .service
        .create_entry(ctx.clone(), input("tamper"))
        .await
        .expect("create");
    fixture
        .kernel
        .context()
        .db
        .lock()
        .execute(
            "UPDATE password_vault_fields SET ciphertext = ?2 WHERE entry_id = ?1 AND field_id = 5",
            params![created.id, vec![0x42u8; 8]],
        )
        .expect("inject fictional corruption");
    assert!(matches!(
        fixture
            .service
            .reveal_password(ctx.clone(), &created.id)
            .await,
        Err(VaultError::DataIntegrityError)
    ));
    assert!(fixture.service.status().locked);
    assert!(matches!(
        fixture.service.list_entries(ctx, None, None, None).await,
        Err(VaultError::Locked)
    ));
}

#[tokio::test]
async fn repeated_nonce_and_random_failure_abort_without_second_ciphertext() {
    let repeated = Fixture::with_random(Arc::new(CountingRandom::repeating(14, 6)));
    repeated.initialize().await;
    repeated.unlock().await;
    let ctx = repeated.kernel.context().clone();
    repeated
        .service
        .create_entry(ctx.clone(), input("one"))
        .await
        .expect("first entry");
    assert!(matches!(
        repeated
            .service
            .create_entry(ctx.clone(), input("two"))
            .await,
        Err(VaultError::NonceReuse)
    ));
    assert_eq!(entry_count(&repeated), 1);

    let failing = Fixture::with_random(Arc::new(CountingRandom::failing_at(14)));
    failing.initialize().await;
    failing.unlock().await;
    let ctx = failing.kernel.context().clone();
    failing
        .service
        .create_entry(ctx.clone(), input("one"))
        .await
        .expect("first entry");
    assert!(matches!(
        failing
            .service
            .create_entry(ctx.clone(), input("two"))
            .await,
        Err(VaultError::RandomSourceFailed)
    ));
    assert_eq!(entry_count(&failing), 1);

    let init_failure = Fixture::with_random(Arc::new(CountingRandom::failing_at(2)));
    assert!(matches!(
        init_failure
            .service
            .initialize(
                init_failure.kernel.context().clone(),
                Zeroizing::new(MASTER.to_string()),
                Zeroizing::new(MASTER.to_string()),
            )
            .await,
        Err(VaultError::RandomSourceFailed)
    ));
    assert!(!init_failure.service.status().initialized);
}

fn entry_count(fixture: &Fixture) -> i64 {
    fixture
        .kernel
        .context()
        .db
        .lock()
        .query_row("SELECT COUNT(*) FROM password_vault_entries", [], |row| {
            row.get(0)
        })
        .expect("count entries")
}

#[tokio::test]
async fn master_password_change_rewraps_same_dek_and_forces_lock() {
    let fixture = Fixture::unlocked().await;
    let ctx = fixture.kernel.context().clone();
    let created = fixture
        .service
        .create_entry(ctx.clone(), input("rewrap"))
        .await
        .expect("create");
    let before: Vec<u8> = fixture
        .kernel
        .context()
        .db
        .lock()
        .query_row(
            "SELECT ciphertext FROM password_vault_fields WHERE entry_id = ?1 AND field_id = 5",
            params![created.id],
            |row| row.get(0),
        )
        .expect("ciphertext before");
    let new_password = "new-fictional-master-password";
    let status = fixture
        .service
        .change_master_password(
            ctx.clone(),
            Zeroizing::new(MASTER.to_string()),
            Zeroizing::new(new_password.to_string()),
        )
        .await
        .expect("change master");
    assert!(status.locked);
    assert!(matches!(
        fixture
            .service
            .unlock_master_password(ctx.clone(), Zeroizing::new(MASTER.to_string()))
            .await,
        Err(VaultError::UnlockFailed)
    ));
    fixture
        .service
        .unlock_master_password(ctx.clone(), Zeroizing::new(new_password.to_string()))
        .await
        .expect("new password");
    let after: Vec<u8> = fixture
        .kernel
        .context()
        .db
        .lock()
        .query_row(
            "SELECT ciphertext FROM password_vault_fields WHERE entry_id = ?1 AND field_id = 5",
            params![created.id],
            |row| row.get(0),
        )
        .expect("ciphertext after");
    assert_eq!(before, after, "master rewrap must not rotate field DEK");
    assert_eq!(
        fixture
            .service
            .reveal_password(ctx, &created.id)
            .await
            .expect("read after rewrap")
            .as_str(),
        "not-a-real-password-rewrap"
    );
}

#[tokio::test]
async fn real_idle_values_and_capability_failures_control_locking() {
    let fixture = Fixture::unlocked().await;
    fixture
        .watcher
        .idle
        .store(179, std::sync::atomic::Ordering::SeqCst);
    fixture.module.sample_now();
    assert!(!fixture.service.status().locked);
    let _ = fixture.service.status();
    fixture.module.sample_now();
    assert!(
        !fixture.service.status().locked,
        "IPC must not refresh or delay idle source"
    );
    fixture
        .watcher
        .idle
        .store(180, std::sync::atomic::Ordering::SeqCst);
    fixture.module.sample_now();
    assert!(fixture.service.status().locked);

    fixture.unlock().await;
    fixture
        .watcher
        .idle
        .store(0, std::sync::atomic::Ordering::SeqCst);
    fixture
        .watcher
        .fail
        .store(true, std::sync::atomic::Ordering::SeqCst);
    fixture.module.sample_now();
    assert!(fixture.service.status().locked);
    assert!(fixture.service.status().capability_error.is_some());

    fixture
        .watcher
        .fail
        .store(false, std::sync::atomic::Ordering::SeqCst);
    fixture.unlock().await;
    fixture
        .watcher
        .locked
        .store(true, std::sync::atomic::Ordering::SeqCst);
    fixture.module.sample_now();
    assert!(fixture.service.status().locked);
    fixture.module.notify_sleep();
    fixture.module.notify_page_close();
    fixture.module.notify_screen_lock();
    assert!(fixture.service.status().locked);
}

#[tokio::test]
async fn lock_invalidates_a_late_unlock_result() {
    let fixture = Fixture::initialized().await;
    let service = Arc::clone(&fixture.service);
    let ctx = fixture.kernel.context().clone();
    let pending = tokio::spawn(async move {
        service
            .unlock_master_password(ctx, Zeroizing::new(MASTER.to_string()))
            .await
    });
    for _ in 0..100 {
        if fixture.service.status().unlocking {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert!(fixture.service.status().unlocking);
    fixture.service.lock(LockReason::Manual);
    assert!(matches!(
        pending.await.expect("join unlock"),
        Err(VaultError::Locked)
    ));
    assert!(fixture.service.status().locked);
}

#[tokio::test]
async fn disabled_module_rejects_calls_and_restart_stays_locked() {
    let fixture = Fixture::unlocked().await;
    fixture
        .kernel
        .set_module_enabled(crate::MODULE_ID, false)
        .expect("disable");
    assert!(matches!(
        fixture
            .service
            .list_entries(fixture.kernel.context().clone(), None, None, None)
            .await,
        Err(VaultError::Disabled)
    ));
    fixture
        .kernel
        .set_module_enabled(crate::MODULE_ID, true)
        .expect("enable");
    assert!(fixture.service.status().locked);
}

#[test]
fn migration_preserves_legacy_tables_and_checksum_changes_fail_closed() {
    let fixture = Fixture::new();
    let conn = fixture.kernel.context().db.lock();
    conn.execute(
        "CREATE TABLE legacy_fixture (id INTEGER PRIMARY KEY, value TEXT)",
        [],
    )
    .expect("legacy table");
    conn.execute("INSERT INTO legacy_fixture(value) VALUES ('keep')", [])
        .expect("legacy row");
    drop(conn);
    assert_eq!(
        fixture
            .kernel
            .context()
            .db
            .run_migrations(&store::migrations())
            .expect("idempotent migration"),
        0
    );
    let conn = fixture.kernel.context().db.lock();
    assert_eq!(
        conn.query_row("SELECT value FROM legacy_fixture WHERE id = 1", [], |row| {
            row.get::<_, String>(0)
        })
        .expect("legacy data"),
        "keep"
    );
    conn.execute(
        "UPDATE _schema_migrations SET checksum = 'tampered' WHERE module_id = 'password_vault' AND version = 1",
        [],
    )
    .expect("inject checksum change");
    assert!(matches!(
        store::validate_storage(&conn),
        Err(VaultError::UnsupportedFormat)
    ));
}

#[test]
fn input_limits_and_generator_policy_fail_closed() {
    let fixture = Fixture::new();
    let mut oversized = input("limits");
    oversized.password = "x".repeat(64 * 1024 + 1);
    assert_eq!(oversized.validate(), Err("field exceeds 64 KiB"));
    assert!(matches!(
        fixture.service.generate_password(PasswordPolicy {
            length: 7,
            ..PasswordPolicy::default()
        }),
        Err(VaultError::InvalidInput(_))
    ));
    assert!(matches!(
        fixture.service.generate_password(PasswordPolicy {
            length: 20,
            uppercase: false,
            lowercase: false,
            digits: false,
            symbols: false,
        }),
        Err(VaultError::InvalidInput(_))
    ));
}
