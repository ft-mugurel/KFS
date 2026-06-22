use crate::sched::scheduler::{CURRENT_PID, PROCESS_TABLE};
use crate::sched::task::ContextFrame;

pub(super) unsafe fn syscall_exit(regs: *mut ContextFrame) {
    unsafe {
        let exit_code = (*regs).arg1();
        let current_pid = CURRENT_PID;
        crate::pr_info!("PID {} exited with code {}\n", current_pid, exit_code);

        let task = PROCESS_TABLE[current_pid].as_mut().unwrap();
        task.state = crate::sched::task::ProcessState::Zombie;
        let old_cr3 = task.context.cr3;
        let boot_cr3 = crate::paging::page_table::bootstrap_directory_phys_addr();

        crate::paging::physical::free_physical_page(old_cr3);
        crate::x86::write_cr3(boot_cr3);
        loop {
            core::arch::asm!("sti; hlt");
        }
    }
}

pub(super) unsafe fn syscall_wait(regs: *mut ContextFrame) {
    unsafe {
        let current_pid = CURRENT_PID;
        for i in 1..crate::sched::scheduler::MAX_PROCESSES {
            if let Some(ref mut child) = PROCESS_TABLE[i] {
                if child.family.parent_pid == current_pid as u32
                    && child.state == crate::sched::task::ProcessState::Zombie
                {
                    crate::pr_info!(
                        "Parent PID {} reaped Zombie PID {}\n",
                        current_pid,
                        child.pid
                    );
                    let child_pid = child.pid;
                    PROCESS_TABLE[i] = None;
                    (*regs).set_return_value(child_pid);
                    return;
                }
            }
        }
        (*regs).set_return_value(0);
    }
}
