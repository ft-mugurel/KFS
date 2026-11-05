use super::args::syscall_define;
use crate::error::KernelError;
use crate::interrupts::timer;
use crate::sched::{self, ProcessState};

syscall_define! {
    pub(super) fn syscall_nanosleep(sleep_ms: u32) {
        let wakeup_tick = timer::get_ticks() as u64 + timer::ms_to_ticks(sleep_ms);
        // SAFETY: a syscall always runs on a task's kernel stack, so `current()`
        // is valid. The reference ends here, before anything can block.
        let task = unsafe { sched::current().as_mut() }.ok_or(KernelError::Srch)?;
        task.wakeup_time = wakeup_tick;
        sched::set_state(task, ProcessState::Sleeping);

        // SAFETY: as `state` is re-read because a wakeup on another CPU
        // changes it. Interrupts are enabled so the wakeup can be delivered.
        while unsafe { sched::current().as_ref() }
            .is_some_and(|task| task.state == ProcessState::Sleeping)
        {
            unsafe { core::arch::asm!("sti; hlt") };
        }
        Ok(0)
    }
}
