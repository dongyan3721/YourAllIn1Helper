//! 键盘、鼠标输入能力的原生实现。
use super::{native_error, NativeOsHal};
use crate::{HalResult, InputSimulator};
use enigo::{Axis, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use std::{sync::atomic::Ordering, time::Duration};

impl NativeOsHal {
    fn input_connection(&self) -> HalResult<Enigo> {
        self.check_input_permission()?;
        // Permission requests belong to an explicit setup action, never every paste/scroll.
        let settings = Settings {
            open_prompt_to_get_permissions: false,
            ..Settings::default()
        };
        Enigo::new(&settings).map_err(native_error)
    }

    fn send_modified_key(&self, key_char: char) -> HalResult<()> {
        #[cfg(target_os = "macos")]
        let keycode = mac_shortcut_keycode(key_char)?;
        let mut enigo = self.input_connection()?;
        #[cfg(target_os = "macos")]
        let modifier = Key::Meta;
        #[cfg(not(target_os = "macos"))]
        let modifier = Key::Control;
        enigo
            .key(modifier, Direction::Press)
            .map_err(native_error)?;
        // Cmd+C/V are physical shortcuts, not text entry. Avoid TIS keyboard-layout
        // discovery (and input-method data access) for these fixed macOS key codes.
        #[cfg(target_os = "macos")]
        let key_result = enigo.raw(keycode, Direction::Click);
        #[cfg(not(target_os = "macos"))]
        let key_result = enigo.key(Key::Unicode(key_char), Direction::Click);
        let release_result = enigo
            .key(modifier, Direction::Release)
            .map_err(native_error);
        key_result.map_err(native_error)?;
        release_result?;
        Ok(())
    }
}

impl InputSimulator for NativeOsHal {
    fn check_input_permission(&self) -> HalResult<()> {
        #[cfg(target_os = "macos")]
        if !super::selection::macos::is_accessibility_trusted() {
            return Err(crate::HalError::PermissionDenied {
                capability: "accessibility",
                guidance: "请在「系统设置 → 隐私与安全 → 设备控制和数据访问」（旧版称「辅助功能」）中添加当前运行的 App。旧测试副本的开关不代表当前签名副本已获授权；重新添加当前 App 后重启再试。".to_string(),
            });
        }
        Ok(())
    }

    fn paste_trigger_count(&self) -> u64 {
        self.paste_triggers.load(Ordering::SeqCst) as u64
    }
    fn send_paste_shortcut(&self) -> HalResult<()> {
        self.check_input_permission()?;
        // Give the previous application time to regain focus after the transient popup closes.
        std::thread::sleep(Duration::from_millis(140));
        self.send_modified_key('v')?;
        self.paste_triggers.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn send_copy_shortcut(&self) -> HalResult<()> {
        self.send_modified_key('c')
    }

    fn scroll_wheel_at(&self, x: i32, y: i32, delta_y: i32) -> HalResult<()> {
        let mut enigo = self.input_connection()?;
        enigo
            .move_mouse(x, y, Coordinate::Abs)
            .map_err(native_error)?;
        enigo.scroll(delta_y, Axis::Vertical).map_err(native_error)
    }
}

#[cfg(target_os = "macos")]
fn mac_shortcut_keycode(key: char) -> HalResult<u16> {
    match key {
        'c' => Ok(0x08),
        'v' => Ok(0x09),
        _ => Err(crate::HalError::Unsupported("mac_shortcut_key")),
    }
}
