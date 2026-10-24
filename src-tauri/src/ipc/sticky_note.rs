//! 便笺 IPC 适配。
use super::*;

impl DesktopHost {
    pub(super) fn dispatch_sticky_note(&self, cmd: &str, args: Value) -> CoreResult<Value> {
        let ctx = self.kernel.context();
        match cmd {
            "sticky_create_note" => {
                let title = args.get("title").and_then(Value::as_str);
                let content_md = args.get("content_md").and_then(Value::as_str);
                let theme_color = args.get("theme_color").and_then(Value::as_str);
                let note = self
                    .sticky_note
                    .create_note(ctx, title, content_md, theme_color)?;
                Ok(serde_json::to_value(note)?)
            }

            "sticky_list_notes" => {
                let only_open = args
                    .get("only_open")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let notes = self.sticky_note.list_notes(ctx, only_open)?;
                Ok(serde_json::to_value(notes)?)
            }

            "sticky_get_note" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                let note = self.sticky_note.get_note(ctx, id)?;
                Ok(serde_json::to_value(note)?)
            }

            "sticky_open_note" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                let note = self.sticky_note.open_note(ctx, id)?;
                Ok(serde_json::to_value(note)?)
            }

            "sticky_save_content" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                let title = args.get("title").and_then(Value::as_str);
                let content_md = args.get("content_md").and_then(Value::as_str).unwrap_or("");
                let updated = self.sticky_note.save_content(ctx, id, title, content_md)?;
                Ok(serde_json::to_value(updated)?)
            }

            "sticky_update_window_state" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                let current = self.sticky_note.get_note(ctx, id)?.ok_or_else(|| {
                    app_core::CoreError::Internal(anyhow::anyhow!("Note '{}' not found", id))
                })?;
                let update = StickyNoteWindowStateUpdate {
                    pos_x: args
                        .get("pos_x")
                        .and_then(Value::as_i64)
                        .map(|v| v as i32)
                        .or(current.pos_x),
                    pos_y: args
                        .get("pos_y")
                        .and_then(Value::as_i64)
                        .map(|v| v as i32)
                        .or(current.pos_y),
                    width: args
                        .get("width")
                        .and_then(Value::as_u64)
                        .map(|v| v as u32)
                        .unwrap_or(current.width),
                    height: args
                        .get("height")
                        .and_then(Value::as_u64)
                        .map(|v| v as u32)
                        .unwrap_or(current.height),
                    is_pinned: args
                        .get("is_pinned")
                        .and_then(Value::as_bool)
                        .unwrap_or(current.is_pinned),
                    opacity: args
                        .get("opacity")
                        .and_then(Value::as_f64)
                        .unwrap_or(current.opacity),
                };
                self.sticky_note.update_window_state(ctx, id, update)?;
                let refreshed = self.sticky_note.get_note(ctx, id)?;
                Ok(serde_json::to_value(refreshed)?)
            }

            "sticky_update_theme" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                let theme_color = args
                    .get("theme_color")
                    .and_then(Value::as_str)
                    .unwrap_or("amber");
                ctx.db.with_conn(|conn| {
                    conn.execute(
                        "UPDATE sticky_notes SET theme_color = ?1 WHERE id = ?2",
                        [theme_color, id],
                    )?;
                    Ok(())
                })?;
                let refreshed = self.sticky_note.get_note(ctx, id)?;
                Ok(serde_json::to_value(refreshed)?)
            }

            "sticky_close_note" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                self.sticky_note.close_note(ctx, id)?;
                Ok(json!({
                    "id": id,
                    "closed": true,
                    "active_window_count": ctx.window_pool.active_window_count()
                }))
            }

            "sticky_delete_note" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                let deleted = self.sticky_note.delete_note(ctx, id)?;
                Ok(json!({
                    "id": id,
                    "deleted": deleted,
                    "active_window_count": ctx.window_pool.active_window_count()
                }))
            }

            "sticky_export_markdown" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                let markdown = self.sticky_note.export_markdown(ctx, id)?;
                Ok(json!({ "id": id, "markdown": markdown }))
            }

            _ => Err(CoreError::Config(format!("Unknown IPC command: {}", cmd))),
        }
    }
}
