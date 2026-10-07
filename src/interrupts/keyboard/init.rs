use crate::{
    interrupts::{
        idt::register_interrupt_handler,
        keyboard::{
            KeyCode, KeyEvent, Modifiers, decode_set1_scancode, keycode_to_char, push_char,
            toggle_layout,
        },
    },
    locks::Spinlock,
    signals::Signal,
    smp::ipi::request_shutdown,
    startup_config::pic,
    vga::text_mod::{
        disable_cursor, enable_cursor, scroll_view_down, scroll_view_to_bottom, scroll_view_to_top,
        scroll_view_up, set_big_cursor, set_cursor_shape, set_small_cursor, switch_screen,
        switch_to_next_screen, switch_to_previous_screen,
    },
    x86::{inb, outb},
};

static mut EXTENDED_SCANCODE: bool = false;
static mut MODIFIERS: Modifiers = Modifiers::empty();

const SCANCODE_EXTENDED_PREFIX: u8 = 0xE0;
const PIC_KEYBOARD_DATA_PORT: u16 = pic::KEYBOARD_DATA_PORT;
const PIC_MASTER_COMMAND_PORT: u16 = pic::MASTER_COMMAND_PORT;
const PIC_EOI: u8 = pic::EOI;

const KEYBOARD_IRQ_VECTOR: u8 = pic::KEYBOARD_IRQ_VECTOR;

fn handle_key_press(event: KeyEvent, modifiers: Modifiers) {
    crate::security::mix_entropy();

    if modifiers.shift() && modifiers.alt() {
        if event.key == KeyCode::LeftShift || event.key == KeyCode::LeftAlt {
            toggle_layout();
            return;
        }
    }

    match event.key {
        KeyCode::Delete if modifiers.ctrl() && modifiers.alt() => {
            unsafe { request_shutdown() };
            return;
        }
        KeyCode::C if modifiers.ctrl() => {
            unsafe {
                if let Some(task) = crate::sched::current().as_mut() {
                    if task.pid > 0 {
                        task.signals.push(Signal::SIGINT as u8);
                    }
                }
            }
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

    match event.key {
        KeyCode::Backspace => {
            push_char('\x08');
            return;
        }
        KeyCode::Enter => {
            push_char('\n');
            return;
        }
        KeyCode::Tab => {
            push_char('\t');
            return;
        }
        _ => {}
    }

    if let Some(c) = keycode_to_char(event.key, modifiers) {
        push_char(c);
        return;
    }

    match event.key {
        KeyCode::ArrowUp => {
            if modifiers.shift() {
                scroll_view_up();
            } else {
                push_str("\x1b[A");
            }
        }
        KeyCode::ArrowDown => {
            if modifiers.shift() {
                scroll_view_down();
            } else {
                push_str("\x1b[B");
            }
        }
        KeyCode::ArrowLeft => {
            if modifiers.shift() {
                switch_to_previous_screen();
            } else {
                push_str("\x1b[D");
            }
        }
        KeyCode::ArrowRight => {
            if modifiers.shift() {
                switch_to_next_screen();
            } else {
                push_str("\x1b[C");
            }
        }
        KeyCode::Home => {
            push_str("\x1b[H");
        }
        KeyCode::End => {
            push_str("\x1b[F");
        }
        KeyCode::Delete => {
            push_str("\x1b[3~");
        }
        _ => {}
    }
}

fn push_str(s: &str) {
    for c in s.chars() {
        push_char(c);
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
    while (inb(pic::KEYBOARD_COMMAND_PORT) & 0x01) != 0 {
        let sc = inb(pic::KEYBOARD_DATA_PORT);
        push_scancode(sc);
    }
    process_keyboard_event();
    outb(PIC_MASTER_COMMAND_PORT, PIC_EOI);
    crate::smp::lapic::send_eoi();
}

unsafe extern "C" {
    fn isr_keyboard(); // the ISR we defined in NASM
}

#[unsafe(link_section = ".init.text")]
pub fn init_keyboard() {
    register_interrupt_handler(KEYBOARD_IRQ_VECTOR, isr_keyboard); // IRQ1 = IDT index 32 + 1 = 33
    while (inb(pic::KEYBOARD_COMMAND_PORT) & 0x1) != 0 {
        inb(pic::KEYBOARD_DATA_PORT);
    }
}
