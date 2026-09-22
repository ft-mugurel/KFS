use crate::interrupts::timer;
use crate::sched::{self, ContextFrame, ProcessState};
use crate::startup_config::power::CONFIG_HZ;

pub(super) unsafe fn syscall_nanosleep(regs: *mut ContextFrame) {
    unsafe {
        let sleep_ms = (*regs).arg1();
        let ticks_to_wait = (sleep_ms) * (CONFIG_HZ) / 1000;
        let wakeup_tick = timer::get_ticks() as u64 + ticks_to_wait as u64;
        let task = sched::current().as_mut().unwrap();
        task.wakeup_time = wakeup_tick as u64;
        task.state = ProcessState::Sleeping;

        while sched::current().as_ref().unwrap().state == ProcessState::Sleeping {
            core::arch::asm!("sti; hlt");
        }
    }
}
