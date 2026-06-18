use crate::x86::outb;
use crate::startup_config::pic;

const PIT_MODE_SQUARE_WAVE: u8 = 0x36;
const PIT_BASE_FREQUENCY: u32 = 1193182;

pub fn init_pit(target_hz: u32) {
    let divisor = PIT_BASE_FREQUENCY / target_hz;

    let divisor = if divisor > 65535 {
        65535
    } else if divisor < 1 {
        1
    } else {
        divisor as u16
    };

    outb(pic::PIT_COMMAND_PORT, PIT_MODE_SQUARE_WAVE);
    outb(pic::PIT_CHANNEL0_PORT, (divisor & 0xFF) as u8);
    outb(pic::PIT_CHANNEL0_PORT, ((divisor >> 8) & 0xFF) as u8);
}
