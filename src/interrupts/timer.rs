use crate::interrupts::idt::register_interrupt_handler;
use crate::interrupts::pit::init_pit;
use crate::pr_debug;
use crate::signals::process_scheduled_signals;
use crate::startup_config::pic;
use crate::x86::outb;

static mut TICKS: u64 = 0;

unsafe extern "C" {
    fn isr_timer();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn timer_interrupt_handler() {
    unsafe {
        TICKS = TICKS.wrapping_add(1);
    }

    process_scheduled_signals();

    // Send End of Interrupt (EOI) to master PIC
    outb(pic::MASTER_COMMAND_PORT, pic::EOI);
}

pub fn init_timer() {
    pr_debug!(
        "Initializing timer with {} Hz frequency\n",
        crate::startup_config::power::CONFIG_HZ
    );
    init_pit(crate::startup_config::power::CONFIG_HZ);
    register_interrupt_handler(pic::TIMER_IRQ_VECTOR, isr_timer);
}
