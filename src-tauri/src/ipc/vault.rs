//! 密码本专用异步 IPC；KDF、数据库与加解密均不进入同步 AppKit 调用链。
use super::*;
use mod_password_vault::{LockReason, PasswordPolicy, VaultEntryInput, MODULE_ID};
use zeroize::Zeroizing;

#[derive(Deserialize)]
struct InitializeArgs {
    password: String,
    confirmation: String,
}

#[derive(Deserialize)]
struct PasswordArgs {
    password: String,
}

#[derive(Deserialize)]
struct ChangePasswordArgs {
    old_password: String,
    new_password: String,
}

#[derive(Deserialize)]
struct EntryArgs {
    id: String,
}

#[derive(Deserialize)]
struct DeleteArgs {
    id: String,
    expected_version: u64,
}

#[derive(Deserialize)]
struct UpsertArgs {
    id: Option<String>,
    expected_version: Option<u64>,
    input: VaultEntryInput,
}

#[derive(Deserialize)]
struct ListArgs {
    search: Option<String>,
    cursor: Option<String>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
struct GenerateArgs {
    policy: PasswordPolicy,
}

fn parse<T: for<'de> Deserialize<'de>>(args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|error| format!("Invalid vault IPC payload: {}", error))
}

fn vault_error(error: mod_password_vault::VaultError) -> String {
    error.to_string()
}

/// 用共享租约挡住剪贴板历史，30 秒后仅当内容仍是这次写入时才清除。
fn copy_secret_with_lease(
    ctx: &app_core::ModuleContext,
    secret: &str,
) -> Result<(), String> {
    let lease = ctx.event_bus.acquire_clipboard_suppression(
        "password_vault",
        "secret_copy",
        std::time::Duration::from_secs(35),
    );
    let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
    clipboard
        .set_text(secret)
        .map_err(|error| error.to_string())?;
    let expected = secret.to_string();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(30));
        if let Ok(mut board) = arboard::Clipboard::new() {
            if board.get_text().ok().as_deref() == Some(expected.as_str()) {
                let _ = board.clear();
            }
        }
        drop(lease);
    });
    Ok(())
}

fn to_json(value: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| error.to_string())
}

impl DesktopHost {
    async fn dispatch_vault(&self, cmd: &str, args: Value) -> Result<Value, String> {
        // 外层模块开关只负责路由门禁；服务在每个敏感操作中再次检查 accepting/locked/epoch。
        let (service, ctx) = self
            .kernel
            .registry()
            .with_enabled(MODULE_ID, || {
                Ok((self.password_vault.service(), self.kernel.context().clone()))
            })
            .map_err(|error| error.to_string())?;
        match cmd {
            "vault_get_status" => to_json(service.status()),
            "vault_lock" => to_json(service.lock(LockReason::Manual)),
            "vault_initialize" => {
                let args = parse::<InitializeArgs>(args)?;
                let status = service
                    .initialize(
                        ctx,
                        Zeroizing::new(args.password),
                        Zeroizing::new(args.confirmation),
                    )
                    .await
                    .map_err(vault_error)?;
                to_json(status)
            }
            "vault_unlock_master_password" => {
                let args = parse::<PasswordArgs>(args)?;
                let status = service
                    .unlock_master_password(ctx, Zeroizing::new(args.password))
                    .await
                    .map_err(vault_error)?;
                to_json(status)
            }
            "vault_list_entries" => {
                let args = parse::<ListArgs>(args)?;
                let page = service
                    .list_entries(ctx, args.search, args.cursor, args.limit)
                    .await
                    .map_err(vault_error)?;
                to_json(page)
            }
            "vault_get_entry" => {
                let args = parse::<EntryArgs>(args)?;
                let entry = service
                    .get_entry(ctx, &args.id)
                    .await
                    .map_err(vault_error)?;
                to_json(entry)
            }
            "vault_reveal_password" => {
                let args = parse::<EntryArgs>(args)?;
                let password = service
                    .reveal_password(ctx, &args.id)
                    .await
                    .map_err(vault_error)?;
                to_json(password)
            }
            "vault_copy_password" => {
                let args = parse::<EntryArgs>(args)?;
                let password = service
                    .reveal_password(ctx.clone(), &args.id)
                    .await
                    .map_err(vault_error)?;
                copy_secret_with_lease(&ctx, password.as_str())?;
                Ok(json!({ "copied": true, "clear_after_secs": 30 }))
            }
            "vault_upsert_entry" => {
                let args = parse::<UpsertArgs>(args)?;
                let result = match args.id {
                    Some(id) => {
                        let expected = args
                            .expected_version
                            .ok_or_else(|| "expected_version is required for update".to_string())?;
                        service
                            .update_entry(ctx, &id, expected, args.input)
                            .await
                            .map_err(vault_error)?
                    }
                    None => service
                        .create_entry(ctx, args.input)
                        .await
                        .map_err(vault_error)?,
                };
                to_json(result)
            }
            "vault_delete_entry" => {
                let args = parse::<DeleteArgs>(args)?;
                service
                    .delete_entry(ctx, &args.id, args.expected_version)
                    .await
                    .map_err(vault_error)?;
                Ok(json!({ "deleted": true }))
            }
            "vault_generate_password" => {
                let args = parse::<GenerateArgs>(args)?;
                let generated = service
                    .generate_password(args.policy)
                    .map_err(vault_error)?;
                to_json(generated.as_str())
            }
            "vault_change_master_password" => {
                let args = parse::<ChangePasswordArgs>(args)?;
                let status = service
                    .change_master_password(
                        ctx,
                        Zeroizing::new(args.old_password),
                        Zeroizing::new(args.new_password),
                    )
                    .await
                    .map_err(vault_error)?;
                to_json(status)
            }
            _ => Err(
                "feature is disabled in M1: unknown or unavailable vault IPC command".to_string(),
            ),
        }
    }

    pub async fn invoke_vault_ipc(&self, cmd: &str, args: Value) -> IpcResponse {
        match self.dispatch_vault(cmd, args).await {
            Ok(value) => IpcResponse::success(value),
            Err(error) => IpcResponse::err(error),
        }
    }
}
