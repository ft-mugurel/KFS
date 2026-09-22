use super::InputMode;

use core::sync::atomic::{AtomicU8, Ordering};

static INPUT_MODE: AtomicU8 = AtomicU8::new(InputMode::EventDriven as u8);

pub fn get_input_mode() -> InputMode {
    match INPUT_MODE.load(Ordering::SeqCst) {
        0 => InputMode::EventDriven,
        1 => InputMode::Blocking,
        _ => unreachable!(),
    }
}

pub fn push_char(c: char) {
    crate::tty::push_char(c);
}
