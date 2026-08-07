use crate::interrupts::timer;
use crate::sched::{self, ContextFrame, ProcessState};
use crate::startup_config::power::CONFIG_HZ;

pub(super) unsafe fn syscall_nanosleep(regs: *mut ContextFrame) {
    unsafe {
        let sleep_ms = (*regs).arg1();
        let ticks_to_wait = (sleep_ms as u64) * (CONFIG_HZ as u64) / 1000;
        let wakeup_tick = timer::get_ticks() + ticks_to_wait;
        let task = sched::current().as_mut().unwrap();
        task.wakeup_time = wakeup_tick;
        task.state = ProcessState::Sleeping;

        while sched::current().as_ref().unwrap().state == ProcessState::Sleeping {
            core::arch::asm!("sti; hlt");
        }
    }
}
