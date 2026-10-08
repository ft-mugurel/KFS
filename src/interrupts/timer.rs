use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::{
    interrupts::{idt::register_interrupt_handler, pit::init_pit},
    pr_debug, sched, smp,
    startup_config::{pic, power::CONFIG_HZ},
    x86::outb,
};

static TICKS: AtomicUsize = AtomicUsize::new(0);
static mut INITIAL_TSC: u64 = 0;
static mut LAST_TSC: u64 = 0;

/// Set to true once the PIT is masked and the BSP's LAPIC timer takes over
static PIT_MASKED: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
    fn isr_timer();
    fn isr_lapic_timer();
}


#[unsafe(no_mangle)]
pub unsafe extern "C" fn timer_interrupt_handler(old_esp: u32) -> u32 {
    TICKS.fetch_add(1, Ordering::Relaxed);

    outb(pic::MASTER_COMMAND_PORT, pic::EOI);
    smp::lapic::send_eoi();
    schedule_timer_interrupt(old_esp)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lapic_timer_interrupt_handler(old_esp: u32) -> u32 {
    if PIT_MASKED.load(Ordering::Relaxed) && unsafe { sched::current_cpu() == 0 } {
        TICKS.fetch_add(1, Ordering::Relaxed);
    }
    smp::lapic::send_eoi();
    schedule_timer_interrupt(old_esp)
}

unsafe fn schedule_timer_interrupt(old_esp: u32) -> u32 {
    if !sched::is_scheduler_active() {
        return old_esp;
    }

    if sched::current_pid() < smp::MAX_CPUS as u32
        && !sched::has_runnable_tasks()
        && !sched::wakeup_may_be_due(get_ticks() as u64)
    {
        return old_esp;
    }

    let next_esp = sched::schedule(old_esp);
    if next_esp != old_esp {
        pr_debug!(
            "[TIMER] Pivot old_esp={:#x} -> next_esp={:#x}\n",
            old_esp,
            next_esp
        );
    }
    next_esp
}

#[unsafe(link_section = ".init.text")]
pub fn init_timer() {
    pr_debug!("Initializing timer with {} Hz frequency\n", CONFIG_HZ);
    init_pit(CONFIG_HZ);
    register_interrupt_handler(pic::TIMER_IRQ_VECTOR, isr_timer);
    register_interrupt_handler(smp::lapic::TIMER_VECTOR, isr_lapic_timer);
    unsafe { INITIAL_TSC = get_tsc_delta() };
}

/// Mask the PIT (IRQ0) on the 8259 PIC after LAPIC calibration is complete.
/// The BSP's LAPIC timer will take over
pub fn mask_pit() {
    // Read current mask and set bit 0 to disable IRQ0 (PIT).
    let current_mask = crate::x86::inb(pic::MASTER_DATA_PORT);
    outb(pic::MASTER_DATA_PORT, current_mask | pic::MASK_IRQ0);
    PIT_MASKED.store(true, Ordering::Release);
}

pub fn get_ticks() -> usize {
    TICKS.load(Ordering::Relaxed)
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
