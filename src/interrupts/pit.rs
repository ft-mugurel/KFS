use core::sync::atomic::{AtomicU32, Ordering};

use crate::startup_config::pic;
use crate::x86::outb;

const PIT_MODE_SQUARE_WAVE: u8 = 0x36;
const PIT_BASE_FREQUENCY: u32 = 1193182;

static PIT_DIVISOR: AtomicU32 = AtomicU32::new(0);

#[unsafe(link_section = ".init.text")]
pub unsafe fn init_pit(target_hz: u32) {
    let divisor = (PIT_BASE_FREQUENCY / target_hz.max(1)).clamp(1, 65535) as u16;
    PIT_DIVISOR.store(divisor as u32, Ordering::Relaxed);

    unsafe {
        outb(pic::PIT_COMMAND_PORT, PIT_MODE_SQUARE_WAVE);
        outb(pic::PIT_CHANNEL0_PORT, (divisor & 0xFF) as u8);
        outb(pic::PIT_CHANNEL0_PORT, ((divisor >> 8) & 0xFF) as u8);
    }
}

pub fn frequency_hz() -> u32 {
    let divisor = PIT_DIVISOR.load(Ordering::Relaxed);
    if divisor == 0 {
        return 0;
    }
    (PIT_BASE_FREQUENCY + divisor / 2) / divisor
}

pub fn ticks_to_ms(ticks: u32) -> u32 {
    let divisor = PIT_DIVISOR.load(Ordering::Relaxed) as u64;
    (ticks as u64 * divisor * 1000 / PIT_BASE_FREQUENCY as u64) as u32
}
