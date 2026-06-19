use crate::interrupts::task_queue::execute_tasks;
use crate::spin::Spinlock;
use crate::startup_config::shell::SCREEN_INDEX;
use crate::vga::text_mod::out::{active_cursor_position, active_screen_index, print_char_on, set_cursor_position_on};
use core::arch::asm;

use core::sync::atomic::{AtomicU8, Ordering};

const BUF_SIZE: usize = 256;
const NUM_TTYS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    EventDriven = 0,
    Blocking = 1,
}
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

pub(crate) fn push_char(c: char) {
    let active_tty = active_screen_index();

    let mut buffers = TTY_BUFFERS.lock();
    let buf = &mut buffers[active_tty];

    let next_head = (buf.head + 1) % BUF_SIZE;
    if next_head != buf.tail {
        buf.data[buf.head] = c;
        buf.head = next_head;
    }
}

pub fn get_char_for_tty(tty_id: usize) -> char {
    loop {
        crate::interrupts::task_queue::execute_tasks();

        let c = {
            let mut buffers = TTY_BUFFERS.lock();
            let buf = &mut buffers[tty_id];
            
            if buf.head != buf.tail {
                let val = buf.data[buf.tail];
                buf.tail = (buf.tail + 1) % BUF_SIZE;
                Some(val)
            } else {
                None
            }
        };

        if let Some(ch) = c {
            return ch;
        }

        unsafe { core::arch::asm!("hlt"); }
    }
}

pub fn get_line(tty_id: usize, buffer: &mut [u8]) -> usize {
    set_input_mode(InputMode::Blocking);

    let mut idx = 0;

    loop {
        let c = get_char_for_tty(tty_id);

        if c == '\n' {
            print_char_on(SCREEN_INDEX, '\n');
            break;
        } else if c == '\x08' {
            // Backspace
            if idx > 0 {
                idx -= 1;
                let (x, y) = active_cursor_position();
                if x > 0 {
                    set_cursor_position_on(SCREEN_INDEX, x - 1, y);
                    print_char_on(SCREEN_INDEX, ' ');
                    set_cursor_position_on(SCREEN_INDEX, x - 1, y);
                }
            }
        } else if idx < buffer.len() && c.is_ascii() && !c.is_ascii_control() {
            buffer[idx] = c as u8;
            idx += 1;
            print_char_on(SCREEN_INDEX, c);
        }
    }
    set_input_mode(InputMode::EventDriven);

    idx
}
