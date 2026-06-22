use crate::interrupts::idt::register_interrupt_handler;
use crate::interrupts::pit::init_pit;
use crate::pr_debug;
use crate::sched::scheduler::schedule;
use crate::signals::process_scheduled_signals;
use crate::startup_config::pic;
use crate::x86::outb;

static mut TICKS: u64 = 0;
static mut INITIAL_TSC: u64 = 0;
static mut LAST_TSC: u64 = 0;

unsafe extern "C" {
    fn isr_timer();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn timer_interrupt_handler(old_esp: u32) -> u32 {
    unsafe {
        TICKS = TICKS.wrapping_add(1);
    }

    process_scheduled_signals();

    // Send End of Interrupt (EOI) to master PIC
    outb(pic::MASTER_COMMAND_PORT, pic::EOI);

    schedule(old_esp)
}

pub fn init_timer() {
    pr_debug!(
        "Initializing timer with {} Hz frequency\n",
        crate::startup_config::power::CONFIG_HZ
    );
    init_pit(crate::startup_config::power::CONFIG_HZ);
    register_interrupt_handler(pic::TIMER_IRQ_VECTOR, isr_timer);
    unsafe { INITIAL_TSC = get_tsc_delta() };
}

pub fn get_ticks() -> u64 {
    let ticks = unsafe { TICKS };
    ticks
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
