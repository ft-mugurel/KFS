use crate::error::KernelError;
use crate::paging;
use crate::pr_info;
use crate::sched::{self, ContextFrame, ProcessState, CURRENT_PID, MAX_CHILDREN, PROCESS_TABLE};
use crate::x86;

pub(super) unsafe fn syscall_exit(regs: *mut ContextFrame) {
    unsafe {
        let exit_code = (*regs).arg1();
        let current_pid = CURRENT_PID;
        pr_info!("PID {} exited with code {}\n", current_pid, exit_code);

        let task = PROCESS_TABLE[current_pid].as_mut().unwrap();

        task.state = ProcessState::Zombie;
        task.exit_code = Some(exit_code);

        let init_task = PROCESS_TABLE[1].as_mut().unwrap();
        for i in 0..task.family.child_count {
            let orphan_pid = task.family.children[i];

            if let Some(ref mut orphan) = PROCESS_TABLE[orphan_pid as usize] {
                orphan.family.parent_pid = 1;
            }

            if init_task.family.child_count < MAX_CHILDREN {
                init_task.family.children[init_task.family.child_count] = orphan_pid;
                init_task.family.child_count += 1;
            }
        }
        task.family.child_count = 0;

        let parent_pid = task.family.parent_pid as usize;
        if let Some(ref mut parent) = PROCESS_TABLE[parent_pid] {
            if parent.state == ProcessState::Sleeping {
                parent.state = ProcessState::Ready;
            }
        }

        let old_cr3 = task.context.cr3;
        let boot_cr3 = paging::bootstrap_directory_phys_addr();
        x86::write_cr3(boot_cr3);
        paging::free_user_address_space(old_cr3);

        sched::yield_cpu();
    }
}

pub(super) fn syscall_wait(regs: *mut ContextFrame) {
    unsafe {
        let parent_pid = CURRENT_PID;
        let parent_task = PROCESS_TABLE[parent_pid].as_mut().unwrap();

        if parent_task.family.child_count == 0 {
            (*regs).set_return_error(KernelError::ECHILD);
            return;
        }

        for i in 0..parent_task.family.child_count {
            let child_pid = parent_task.family.children[i] as usize;

            if let Some(ref mut child) = PROCESS_TABLE[child_pid] {
                if child.state == ProcessState::Zombie {
                    let reaped_pid = child.pid;

                    PROCESS_TABLE[child_pid] = None;

                    let last_idx = parent_task.family.child_count - 1;
                    parent_task.family.children[i] = parent_task.family.children[last_idx];
                    parent_task.family.child_count -= 1;

                    pr_info!(
                        "Parent PID {} reaped Zombie PID {}\n",
                        parent_pid,
                        reaped_pid
                    );

                    (*regs).set_return_value(reaped_pid);
                    return;
                }
            }
        }

        parent_task.state = ProcessState::Sleeping;
        (*regs).eip -= 2;
    }
}
