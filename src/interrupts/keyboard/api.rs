use super::InputMode;
use crate::locks::Spinlock;
use crate::vga::text_mod::active_screen_index;

use core::sync::atomic::{AtomicU8, Ordering};

const BUF_SIZE: usize = 256;
const NUM_TTYS: usize = 6;

static INPUT_MODE: AtomicU8 = AtomicU8::new(InputMode::EventDriven as u8);

pub fn set_input_mode(mode: InputMode) {
    INPUT_MODE.store(mode as u8, Ordering::SeqCst);
}

pub fn get_input_mode() -> InputMode {
    match INPUT_MODE.load(Ordering::SeqCst) {
        0 => InputMode::EventDriven,
        1 => InputMode::Blocking,
        _ => unreachable!(),
    }
}

struct KeyboardBuffer {
    data: [char; BUF_SIZE],
    head: usize,
    tail: usize,
}

const EMPTY_BUFFER: KeyboardBuffer = KeyboardBuffer { data: ['\0'; BUF_SIZE], head: 0, tail: 0 };
static TTY_BUFFERS: Spinlock<[KeyboardBuffer; NUM_TTYS]> = Spinlock::new([EMPTY_BUFFER; NUM_TTYS]);

pub fn push_char(c: char) {
    let active_tty = active_screen_index();

    let mut buffers = TTY_BUFFERS.lock();
    let buf = &mut buffers[active_tty];

    let next_head = (buf.head + 1) % BUF_SIZE;
    if next_head != buf.tail {
        buf.data[buf.head] = c;
        buf.head = next_head;
    }
}
