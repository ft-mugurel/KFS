use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::{
    interrupts::{
        idt::register_interrupt_handler,
        pit::{self, init_pit},
        stats::{self, IrqSource},
    },
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

pub const LAPIC_TICK_MS: u32 = 10;

/// PIT (IRQ 0) handler. Returns the stack pointer to resume on, which
/// differs from `old_esp` when the scheduler switches tasks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn timer_interrupt_handler(old_esp: u32) -> u32 {
    let ticks = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    stats::count(IrqSource::Pit);
    unsafe { sched::stats::account_tick(0, old_esp) };
    sched::stats::global_tick(ticks, pit::frequency_hz());

    unsafe {
        outb(pic::MASTER_COMMAND_PORT, pic::EOI);
        smp::lapic::send_eoi();
    }
    crate::modules::dispatch_cpu_tick(0);
    unsafe { schedule_timer_interrupt(old_esp) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lapic_timer_interrupt_handler(old_esp: u32) -> u32 {
    let cpu_id = unsafe { sched::current_cpu() };
    let pit_masked = PIT_MASKED.load(Ordering::Relaxed);
    stats::count(IrqSource::LapicTimer);
    if pit_masked && cpu_id == 0 {
        let ticks = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
        sched::stats::global_tick(ticks, tick_frequency_hz());
    }
    unsafe { smp::lapic::send_eoi() };
    // Until the PIT is masked, CPU 0's ticks are accounted by the PIT handler.
    if pit_masked || cpu_id != 0 {
        unsafe { sched::stats::account_tick(cpu_id, old_esp) };
        crate::modules::dispatch_cpu_tick(cpu_id);
    }
    unsafe { schedule_timer_interrupt(old_esp) }
}

pub fn tick_frequency_hz() -> u32 {
    if PIT_MASKED.load(Ordering::Relaxed) {
        1000 / LAPIC_TICK_MS
    } else {
        pit::frequency_hz()
    }
}

/// for calibrating against the PIT
pub fn pit_ticks_to_ms(ticks: u32) -> u32 {
    pit::ticks_to_ms(ticks)
}

/// TICKS follows the PIT only until it is masked and the LAPIC timer takes over
/// So, CONFIG_HZ is not always valid 
pub fn ms_to_ticks(ms: u32) -> u64 {
    ms as u64 * tick_frequency_hz() as u64 / 1000
}

unsafe fn schedule_timer_interrupt(old_esp: u32) -> u32 {
    if !sched::is_scheduler_active() {
        return old_esp;
    }

    if unsafe { sched::current_pid() } < smp::MAX_CPUS as u32
        && !sched::has_runnable_tasks()
        && !sched::wakeup_may_be_due(get_ticks() as u64)
    {
        return old_esp;
    }

    let next_esp = unsafe { sched::schedule(old_esp) };
    if next_esp != old_esp {
        pr_debug!(
            "[TIMER] Pivot old_esp={:#x} -> next_esp={:#x}",
            old_esp,
            next_esp
        );
    }
    next_esp
}

#[unsafe(link_section = ".init.text")]
pub fn init_timer() {
    pr_debug!("Initializing timer with {} Hz frequency", CONFIG_HZ);
    unsafe { init_pit(CONFIG_HZ) };
    register_interrupt_handler(pic::TIMER_IRQ_VECTOR, isr_timer);
    register_interrupt_handler(smp::lapic::TIMER_VECTOR, isr_lapic_timer);
    stats::mark_installed(IrqSource::Pit);
    stats::mark_installed(IrqSource::LapicTimer);
    unsafe { INITIAL_TSC = get_tsc_delta() };
}

/// Mask the PIT (IRQ0) on the 8259 PIC after LAPIC calibration is complete.
/// The BSP's LAPIC timer will take over
pub unsafe fn mask_pit() {
    // Read current mask and set bit 0 to disable IRQ0 (PIT).
    unsafe {
        let current_mask = crate::x86::inb(pic::MASTER_DATA_PORT);
        outb(pic::MASTER_DATA_PORT, current_mask | pic::MASK_IRQ0);
    }
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

pub fn read_tsc() -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        core::arch::asm!(
            "rdtsc",
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags)
        );
    }
    ((high as u64) << 32) | low as u64
}
