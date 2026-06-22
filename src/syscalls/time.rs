use crate::sched::scheduler::{CURRENT_PID, PROCESS_TABLE};
use crate::sched::task::ContextFrame;

pub(super) unsafe fn syscall_nanosleep(regs: *mut ContextFrame) {
    unsafe {
        let sleep_ms = (*regs).arg1();
        let current_pid = CURRENT_PID;
        let ticks_to_wait =
            (sleep_ms as u64) * (crate::startup_config::power::CONFIG_HZ as u64) / 1000;
        let wakeup_tick = crate::interrupts::timer::get_ticks() + ticks_to_wait;
        let task = PROCESS_TABLE[current_pid].as_mut().unwrap();
        task.wakeup_time = wakeup_tick;
        task.state = crate::sched::task::ProcessState::Sleeping;

        while PROCESS_TABLE[current_pid].as_ref().unwrap().state
            == crate::sched::task::ProcessState::Sleeping
        {
            core::arch::asm!("sti; hlt");
        }
    }
}
