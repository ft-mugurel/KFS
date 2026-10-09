mod api;
mod character_map;
mod keycode;
pub(crate) mod types;

use super::{
    KernelModuleApi, KernelModuleContext, ModuleCallback, ModuleDescriptor, ModuleEvent,
    ModuleEventKind, MODULE_ABI_VERSION,
};
use self::types::{KeyCode, KeyEvent, Modifiers};
use crate::{
    error::KResult,
    interrupts::register_interrupt_handler,
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

pub(crate) use api::push_char;
pub(crate) use character_map::{keycode_to_char, toggle_layout};
pub(crate) use keycode::decode_set1_scancode;

static mut EXTENDED_SCANCODE: bool = false;
static mut MODIFIERS: Modifiers = Modifiers::empty();
static RAW_SCANCODE_QUEUE: Spinlock<[u8; 32]> = Spinlock::new([0; 32]);
static mut QUEUE_HEAD: usize = 0;
static mut QUEUE_TAIL: usize = 0;

const SCANCODE_EXTENDED_PREFIX: u8 = 0xE0;
const PIC_MASTER_COMMAND_PORT: u16 = pic::MASTER_COMMAND_PORT;
const PIC_EOI: u8 = pic::EOI;
const KEYBOARD_IRQ_VECTOR: u8 = pic::KEYBOARD_IRQ_VECTOR;

pub(crate) static DESCRIPTOR: ModuleDescriptor = ModuleDescriptor {
    name: "keyboard",
    version: 1,
    abi_version: MODULE_ABI_VERSION,
    flags: 0,
    init,
    destroy,
    dependencies: &[],
};

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

fn process_scancodes() {
    while let Some(scancode) = pop_scancode() {
        unsafe {
            if scancode == SCANCODE_EXTENDED_PREFIX {
                EXTENDED_SCANCODE = true;
            } else {
                if let Some(event) = decode_set1_scancode(scancode, EXTENDED_SCANCODE) {
                    let mut modifiers = MODIFIERS;
                    modifiers.update_for_event(event);
                    MODIFIERS = modifiers;
                    crate::modules::dispatch_key_event(event, modifiers);
                }
                EXTENDED_SCANCODE = false;
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn keyboard_interrupt_handler() {
    while (inb(pic::KEYBOARD_COMMAND_PORT) & 0x01) != 0 {
        push_scancode(inb(pic::KEYBOARD_DATA_PORT));
    }
    outb(PIC_MASTER_COMMAND_PORT, PIC_EOI);
    crate::smp::lapic::send_eoi();
}

unsafe extern "C" {
    fn isr_keyboard();
}

#[unsafe(link_section = ".init.text")]
pub(crate) fn init_keyboard_irq() {
    register_interrupt_handler(KEYBOARD_IRQ_VECTOR, isr_keyboard);
    while (inb(pic::KEYBOARD_COMMAND_PORT) & 0x1) != 0 {
        inb(pic::KEYBOARD_DATA_PORT);
    }
}

fn init(context: &KernelModuleContext) -> KResult<()> {
    context.api.validate()?;
    (context.api.log)(context.module, "keyboard module received versioned API");
    (context.api.callback_register)(
        context.module,
        ModuleEventKind::KeyPressed,
        callback as ModuleCallback,
    )?;
    (context.api.callback_register)(
        context.module,
        ModuleEventKind::KeyReleased,
        callback as ModuleCallback,
    )?;
    (context.api.callback_register)(
        context.module,
        ModuleEventKind::CpuTick,
        callback as ModuleCallback,
    )?;
    init_keyboard_irq();
    Ok(())
}

fn destroy(_: &KernelModuleContext) {}

fn callback(owner: super::ModuleId, event: ModuleEvent, api: &KernelModuleApi) {
    match event {
        ModuleEvent::CpuTick { cpu_id: 0 } => process_scancodes(),
        ModuleEvent::CpuTick { .. } => {}
        ModuleEvent::KeyPressed(key, modifiers) => {
            handle_key_press(key, modifiers);
        }
        ModuleEvent::KeyReleased(_, _) => {}
    }
}

fn handle_key_press(event: KeyEvent, modifiers: Modifiers) {
    crate::security::mix_entropy();

    if modifiers.shift() && modifiers.alt()
        && (event.key == KeyCode::LeftShift || event.key == KeyCode::LeftAlt)
    {
        toggle_layout();
        return;
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
        KeyCode::F1 => return switch_screen(0),
        KeyCode::F2 => return switch_screen(1),
        KeyCode::F3 => return switch_screen(2),
        KeyCode::F4 => return switch_screen(3),
        KeyCode::F5 => return switch_screen(4),
        KeyCode::F6 => return switch_screen(5),
        KeyCode::F7 => return set_big_cursor(),
        KeyCode::F8 => return set_small_cursor(),
        KeyCode::F9 => return set_cursor_shape(0, 15),
        KeyCode::F10 => return disable_cursor(),
        KeyCode::F11 => return enable_cursor(),
        KeyCode::PageUp => return scroll_view_to_top(),
        KeyCode::PageDown => return scroll_view_to_bottom(),
        _ => {}
    }

    match event.key {
        KeyCode::Backspace => return push_char('\x08'),
        KeyCode::Enter => return push_char('\n'),
        KeyCode::Tab => return push_char('\t'),
        _ => {}
    }

    if let Some(character) = keycode_to_char(event.key, modifiers) {
        push_char(character);
        return;
    }

    match event.key {
        KeyCode::ArrowUp if modifiers.shift() => scroll_view_up(),
        KeyCode::ArrowUp => push_str("\x1b[A"),
        KeyCode::ArrowDown if modifiers.shift() => scroll_view_down(),
        KeyCode::ArrowDown => push_str("\x1b[B"),
        KeyCode::ArrowLeft if modifiers.shift() => switch_to_previous_screen(),
        KeyCode::ArrowLeft => push_str("\x1b[D"),
        KeyCode::ArrowRight if modifiers.shift() => switch_to_next_screen(),
        KeyCode::ArrowRight => push_str("\x1b[C"),
        KeyCode::Home => push_str("\x1b[H"),
        KeyCode::End => push_str("\x1b[F"),
        KeyCode::Delete => push_str("\x1b[3~"),
        _ => {}
    }
}

fn push_str(value: &str) {
    for character in value.chars() {
        push_char(character);
    }
}
