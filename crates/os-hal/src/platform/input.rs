//! 键盘、鼠标输入能力的原生实现。
use super::{native_error, NativeOsHal};
use crate::{HalResult, InputSimulator};
use enigo::{Axis, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use std::{sync::atomic::Ordering, time::Duration};

impl NativeOsHal {
    fn send_modified_key(&self, key_char: char) -> HalResult<()> {
        let mut enigo = Enigo::new(&Settings::default()).map_err(native_error)?;
        #[cfg(target_os = "macos")]
        let modifier = Key::Meta;
        #[cfg(not(target_os = "macos"))]
        let modifier = Key::Control;
        enigo
            .key(modifier, Direction::Press)
            .map_err(native_error)?;
        enigo
            .key(Key::Unicode(key_char), Direction::Click)
            .map_err(native_error)?;
        enigo
            .key(modifier, Direction::Release)
            .map_err(native_error)?;
        Ok(())
    }
}

impl InputSimulator for NativeOsHal {
    fn paste_trigger_count(&self) -> u64 {
        self.paste_triggers.load(Ordering::SeqCst) as u64
    }
    fn send_paste_shortcut(&self) -> HalResult<()> {
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
        let mut enigo = Enigo::new(&Settings::default()).map_err(native_error)?;
        enigo
            .move_mouse(x, y, Coordinate::Abs)
            .map_err(native_error)?;
        enigo.scroll(delta_y, Axis::Vertical).map_err(native_error)
    }
}
