use crate::{
    interrupts::{idt::register_interrupt_handler, pit::init_pit},
    pr_debug,
    sched::schedule,
    signals::process_scheduled_signals,
    startup_config::{pic, power::CONFIG_HZ},
    x86::outb,
};

static mut TICKS: u64 = 0;
static mut INITIAL_TSC: u64 = 0;
static mut LAST_TSC: u64 = 0;

unsafe extern "C" {
    fn isr_timer();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn timer_interrupt_handler(old_esp: u32) -> u32 {
    TICKS = TICKS.wrapping_add(1);

    process_scheduled_signals();

    outb(pic::MASTER_COMMAND_PORT, pic::EOI);

    schedule(old_esp)
}

pub fn init_timer() {
    pr_debug!("Initializing timer with {} Hz frequency\n", CONFIG_HZ);
    init_pit(CONFIG_HZ);
    register_interrupt_handler(pic::TIMER_IRQ_VECTOR, isr_timer);
    unsafe { INITIAL_TSC = get_tsc_delta() };
}

pub fn get_ticks() -> u64 {
    unsafe { TICKS }
}

pub fn get_tsc_delta() -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        core::arch::asm!(
            "rdtsc",
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags)
        );
        LAST_TSC = ((high as u64) << 32) | (low as u64);
        LAST_TSC - INITIAL_TSC
    }
}
