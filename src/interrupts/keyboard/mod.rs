mod keycode;
pub(crate) mod types;

use core::sync::atomic::{AtomicBool, Ordering};

use self::keycode::decode_set1_scancode;
use self::types::Modifiers;
use crate::{
    interrupts::register_interrupt_handler,
    locks::Spinlock,
    modules::{self, KeyboardEvent, ModuleEvent},
    startup_config::pic,
    x86::{inb, outb},
};

const SCANCODE_EXTENDED_PREFIX: u8 = 0xE0;
const EVENT_QUEUE_SIZE: usize = 64;

struct KeyboardState {
    extended: bool,
    modifiers: Modifiers,
    events: [Option<KeyboardEvent>; EVENT_QUEUE_SIZE],
    head: usize,
    tail: usize,
}

impl KeyboardState {
    const fn new() -> Self {
        Self {
            extended: false,
            modifiers: Modifiers::empty(),
            events: [None; EVENT_QUEUE_SIZE],
            head: 0,
            tail: 0,
        }
    }

    fn decode(&mut self, scancode: u8) {
        if scancode == SCANCODE_EXTENDED_PREFIX {
            self.extended = true;
            return;
        }
        let extended = core::mem::replace(&mut self.extended, false);
        let Some(event) = decode_set1_scancode(scancode, extended) else {
            return;
        };
        self.modifiers.update_for_event(event);

        let next = (self.head + 1) % EVENT_QUEUE_SIZE;
        if next == self.tail {
            return;
        }
        self.events[self.head] = Some(KeyboardEvent {
            scancode: if extended {
                ((SCANCODE_EXTENDED_PREFIX as u16) << 8) | scancode as u16
            } else {
                scancode as u16
            },
            key: event.key,
            pressed: event.pressed,
            modifiers: self.modifiers,
        });
        self.head = next;
    }

    fn pop(&mut self) -> Option<KeyboardEvent> {
        if self.head == self.tail {
            return None;
        }
        let event = self.events[self.tail].take();
        self.tail = (self.tail + 1) % EVENT_QUEUE_SIZE;
        event
    }

    fn is_empty(&self) -> bool {
        self.head == self.tail
    }
}

static STATE: Spinlock<KeyboardState> = Spinlock::new(KeyboardState::new());

// This is used to prevent multiple CPUs from trying to deliver events at the
// same time and mixing up the order of them
static DELIVERING: AtomicBool = AtomicBool::new(false);

fn deliver_events() {
    loop {
        if DELIVERING
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            // Another CPU is handling it
            return;
        }
        loop {
            let event = STATE.lock().pop();
            let Some(event) = event else {
                break;
            };
            if event.pressed {
                crate::security::mix_entropy();
                modules::dispatch(ModuleEvent::KeyPressed(event));
            } else {
                modules::dispatch(ModuleEvent::KeyReleased(event));
            }
        }
        DELIVERING.store(false, Ordering::Release);
        if STATE.lock().is_empty() {
            return;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn keyboard_interrupt_handler() {
    crate::interrupts::stats::count(crate::interrupts::stats::IrqSource::Keyboard);
    {
        let mut state = STATE.lock();
        while (unsafe { inb(pic::KEYBOARD_COMMAND_PORT) } & 0x01) != 0 {
            state.decode(unsafe { inb(pic::KEYBOARD_DATA_PORT) });
        }
    }
    unsafe {
        outb(pic::MASTER_COMMAND_PORT, pic::EOI);
        crate::smp::lapic::send_eoi();
    }
    deliver_events();
}

unsafe extern "C" {
    fn isr_keyboard();
}

#[unsafe(link_section = ".init.text")]
pub(crate) unsafe fn init_keyboard() {
    register_interrupt_handler(pic::KEYBOARD_IRQ_VECTOR, isr_keyboard);
    crate::interrupts::stats::mark_installed(crate::interrupts::stats::IrqSource::Keyboard);
    unsafe {
        while (inb(pic::KEYBOARD_COMMAND_PORT) & 0x1) != 0 {
            inb(pic::KEYBOARD_DATA_PORT);
        }
    }
}
