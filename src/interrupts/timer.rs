use core::sync::atomic::AtomicUsize;

use crate::{
    interrupts::{idt::register_interrupt_handler, pit::init_pit},
    pr_debug,
    sched::schedule,
    startup_config::{pic, power::CONFIG_HZ},
    x86::outb,
};

static TICKS: AtomicUsize = AtomicUsize::new(0);
static mut INITIAL_TSC: u64 = 0;
static mut LAST_TSC: u64 = 0;

unsafe extern "C" {
    fn isr_timer();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn timer_interrupt_handler(old_esp: u32) -> u32 {
    TICKS.fetch_add(1, core::sync::atomic::Ordering::SeqCst);

    outb(pic::MASTER_COMMAND_PORT, pic::EOI);
    crate::smp::lapic::send_eoi();

    let next_esp = schedule(old_esp);
    if next_esp != old_esp {
        crate::pr_info!("[TIMER] Pivot old_esp={:#x} -> next_esp={:#x}\n", old_esp, next_esp);
    }
    next_esp
}

pub fn init_timer() {
    pr_debug!("Initializing timer with {} Hz frequency\n", CONFIG_HZ);
    init_pit(CONFIG_HZ);
    register_interrupt_handler(pic::TIMER_IRQ_VECTOR, isr_timer);
    unsafe { INITIAL_TSC = get_tsc_delta() };
}

pub fn get_ticks() -> usize {
    TICKS.load(core::sync::atomic::Ordering::SeqCst)
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
