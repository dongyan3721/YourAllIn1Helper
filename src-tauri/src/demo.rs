//! 显式开发演示数据，不在正常启动时写入用户数据库。
use super::*;
impl DesktopHost {
    /// Seeds initial demo records if the database has no clipboard items or sticky notes yet,
    /// while preserving the Level 0 (0 active WebViews) invariant until the user opens a window
    /// or triggers a shortcut.
    pub fn seed_demo_data_if_empty(&self) -> CoreResult<()> {
        let ctx = self.kernel.context();
        let existing_clip = self.clipboard.list_items(ctx, None, 5)?;
        if self.kernel.registry().is_enabled("clipboard") && existing_clip.is_empty() {
            let item1 = self.clipboard.capture_text(
                ctx,
                "cargo test --workspace --all-targets",
                Some("Terminal"),
            )?;
            if let Some(rec) = item1 {
                // Pin the command as a frequently used snippet
                let _ = self.clipboard.toggle_pin(ctx, rec.id);
            }
            let _ = self.clipboard.capture_text(
                ctx,
                "PrimaryMod+Shift+V 唤醒 Ditto 风格剪贴板弹窗（失焦或粘贴后立即销毁 WebView）",
                Some("Visual Studio Code"),
            )?;
            let _ = self.clipboard.capture_text(
                ctx,
                "https://v2.tauri.app/concept/inter-process-communication/",
                Some("Safari"),
            )?;
        }

        let existing_notes = self.sticky_note.list_notes(ctx, false)?;
        if self.kernel.registry().is_enabled("sticky_note") && existing_notes.is_empty() {
            // Create a sample Markdown + LaTeX sticky note and close it so startup remains at Level 0 (0 WebViews),
            // ready to be opened from Dashboard or created via PrimaryMod+Shift+N.
            let note = self.sticky_note.create_note(
                ctx,
                Some("架构指标与公式速记"),
                Some(
                    "### 核心指标\n- [x] 后台静默 **0 WebView** 常驻 (< 25 MB)\n- [x] SQLite WAL + 自动增量迁移\n\n### 数学公式 (LaTeX)\n行内公式：$E = mc^2$，高斯积分：\n$$\\int_{-\\infty}^{\\infty} e^{-x^2}\\,dx = \\sqrt{\\pi}$$",
                ),
                Some("amber"),
            )?;
            self.sticky_note.close_note(ctx, &note.id)?;
        }

        Ok(())
    }
}
