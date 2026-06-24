use crate::interrupts::idt::register_interrupt_handler;
use crate::interrupts::keyboard::{
    decode_set1_scancode, get_input_mode, keycode_to_char, push_char, toggle_layout, InputMode,
    KeyCode, KeyEvent, Modifiers,
};
use crate::interrupts::request_shutdown;
use crate::sched::schedule_task;
use crate::shell::handle_shell_key_event;
use crate::signals::{send_signal, Signal};
use crate::spin::Spinlock;
use crate::startup_config::pic;
use crate::vga::text_mod::{
    disable_cursor, enable_cursor, move_cursor_down, move_cursor_left, move_cursor_right,
    move_cursor_up, scroll_view_down, scroll_view_to_bottom, scroll_view_to_top, scroll_view_up,
    set_big_cursor, set_cursor_shape, set_small_cursor, switch_screen, switch_to_next_screen,
    switch_to_previous_screen,
};
use crate::x86::{inb, outb};

static mut EXTENDED_SCANCODE: bool = false;
static mut MODIFIERS: Modifiers = Modifiers::empty();

const SCANCODE_EXTENDED_PREFIX: u8 = 0xE0;
const PIC_KEYBOARD_DATA_PORT: u16 = pic::KEYBOARD_DATA_PORT;
const PIC_MASTER_COMMAND_PORT: u16 = pic::MASTER_COMMAND_PORT;
const PIC_EOI: u8 = pic::EOI;

const KEYBOARD_IRQ_VECTOR: u8 = pic::KEYBOARD_IRQ_VECTOR;

fn handle_key_press(event: KeyEvent, modifiers: Modifiers) {
    if modifiers.shift() && modifiers.alt() {
        if event.key == KeyCode::LeftShift || event.key == KeyCode::LeftAlt {
            toggle_layout();
            return;
        }
    }

    match event.key {
        KeyCode::Delete if modifiers.ctrl() && modifiers.alt() => {
            request_shutdown();
            return;
        }
        KeyCode::C if modifiers.ctrl() => {
            send_signal(Signal::SIGINT);
            return;
        }
        KeyCode::F1 => {
            switch_screen(0);
            return;
        }
        KeyCode::F2 => {
            switch_screen(1);
            return;
        }
        KeyCode::F3 => {
            switch_screen(2);
            return;
        }
        KeyCode::F4 => {
            switch_screen(3);
            return;
        }
        KeyCode::F5 => {
            switch_screen(4);
            return;
        }
        KeyCode::F6 => {
            switch_screen(5);
            return;
        }
        KeyCode::F7 => {
            set_big_cursor();
            return;
        }
        KeyCode::F8 => {
            set_small_cursor();
            return;
        }
        KeyCode::F9 => {
            set_cursor_shape(0, 15);
            return;
        }
        KeyCode::F10 => {
            disable_cursor();
            return;
        }
        KeyCode::F11 => {
            enable_cursor();
            return;
        }
        KeyCode::PageUp => {
            scroll_view_to_top();
            return;
        }
        KeyCode::PageDown => {
            scroll_view_to_bottom();
            return;
        }
        _ => {} // Not a global hotkey, continue down to the router
    }

    if get_input_mode() == InputMode::Blocking {
        if !modifiers.has_text_blocking_modifier() {
            if let Some(c) = keycode_to_char(event.key, modifiers) {
                push_char(c);
            }
        }
    } else {
        if handle_shell_key_event(event, modifiers) {
            return;
        }

        match event.key {
            KeyCode::ArrowUp => {
                if modifiers.shift() {
                    scroll_view_up();
                } else {
                    move_cursor_up();
                }
            }
            KeyCode::ArrowDown => {
                if modifiers.shift() {
                    scroll_view_down();
                } else {
                    move_cursor_down();
                }
            }
            KeyCode::ArrowLeft => {
                if modifiers.shift() {
                    switch_to_previous_screen();
                } else {
                    move_cursor_left();
                }
            }
            KeyCode::ArrowRight => {
                if modifiers.shift() {
                    switch_to_next_screen();
                } else {
                    move_cursor_right();
                }
            }
            _ => {}
        }
    }
}

fn process_keyboard_event() {
    while let Some(scancode) = pop_scancode() {
        unsafe {
            if scancode == SCANCODE_EXTENDED_PREFIX {
                EXTENDED_SCANCODE = true;
            } else {
                if let Some(event) = decode_set1_scancode(scancode, EXTENDED_SCANCODE) {
                    let mut modifiers = MODIFIERS;
                    modifiers.update_for_event(event);
                    MODIFIERS = modifiers;
                    if event.pressed {
                        handle_key_press(event, modifiers);
                    }
                }
                EXTENDED_SCANCODE = false;
            }
        }
    }
}

static RAW_SCANCODE_QUEUE: Spinlock<[u8; 32]> = Spinlock::new([0; 32]);
static mut QUEUE_HEAD: usize = 0;
static mut QUEUE_TAIL: usize = 0;

fn push_scancode(code: u8) {
    let mut queue = RAW_SCANCODE_QUEUE.lock();
    unsafe {
        let next_head = (QUEUE_HEAD + 1) % 32;
        if next_head != QUEUE_TAIL {
            queue[QUEUE_HEAD] = code;
            QUEUE_HEAD = next_head;
        }
    }
}

fn pop_scancode() -> Option<u8> {
    let queue = RAW_SCANCODE_QUEUE.lock();
    unsafe {
        if QUEUE_HEAD == QUEUE_TAIL {
            return None;
        }
        let code = queue[QUEUE_TAIL];
        QUEUE_TAIL = (QUEUE_TAIL + 1) % 32;
        Some(code)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn keyboard_interrupt_handler() {
    push_scancode(inb(PIC_KEYBOARD_DATA_PORT));
    schedule_task(process_keyboard_event);
    outb(PIC_MASTER_COMMAND_PORT, PIC_EOI);
}

unsafe extern "C" {
    fn isr_keyboard(); // the ISR we defined in NASM
}

pub fn init_keyboard() {
    register_interrupt_handler(KEYBOARD_IRQ_VECTOR, isr_keyboard); // IRQ1 = IDT index 32 + 1 = 33
    while (inb(pic::KEYBOARD_COMMAND_PORT) & 0x1) != 0 {
        inb(pic::KEYBOARD_DATA_PORT);
    }
}
