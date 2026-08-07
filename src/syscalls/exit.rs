use crate::error::{KResultExt, KernelError};
use crate::fs::{VfsNodeType, OPEN_FILE_TABLE};
use crate::sched::{ContextFrame, ProcessState, MAX_CHILDREN, MAX_FDS_PER_PROCESS, PROCESS_TABLE};
use crate::{ipc, sched};
use crate::{paging, pr_info};

pub unsafe fn syscall_exit(regs: *mut ContextFrame) -> u32 {
    let exit_code = (*regs).arg1();
    let task = sched::current().as_mut().unwrap();

    task.state = ProcessState::Zombie;
    task.exit_code = Some(exit_code);

    // VFS Cleanup
    for i in 0..MAX_FDS_PER_PROCESS {
        if let Some(global_fd) = task.fd_tbl[i] {
            if let Some(open_file) = &mut OPEN_FILE_TABLE[global_fd] {
                if open_file.ref_count > 0 {
                    open_file.ref_count -= 1;
                }

                if open_file.ref_count == 0 {
                    let node = open_file.node;

                    if (*node).node_type == VfsNodeType::Socket {
                        let sock_idx = (*node).inode as usize;
                        ipc::close_socket(sock_idx);
                    }

                    OPEN_FILE_TABLE[global_fd] = None;
                }
            }
            task.fd_tbl[i] = None;
        }
    }

    // Reparent orphans to PID 1
    let mut table = PROCESS_TABLE.lock();
    let mut init_task = table.as_mut_slice()[1].unwrap();
    for i in 0..task.family.child_count {
        let orphan_pid = task.family.children[i];

        if let Some(ref mut orphan) = table[orphan_pid as usize] {
            orphan.family.parent_pid = 1;
        }

        if init_task.family.child_count < MAX_CHILDREN {
            init_task.family.children[init_task.family.child_count] = orphan_pid;
            init_task.family.child_count += 1;
        }
    }
    task.family.child_count = 0;

    // Wake the parent if it is blocked in waitpid()
    let parent_pid = task.family.parent_pid as usize;
    if let Some(ref mut parent) = table[parent_pid] {
        if parent.state == ProcessState::Waiting {
            parent.state = ProcessState::Ready;
        }
    }

    // DO NOT touch CR3 or free memory here.
    // Memory is preserved until the parent calls waitpid().
    let curr_esp = task.context.esp;

    sched::schedule(curr_esp)
}

pub(super) unsafe fn syscall_wait(regs: *mut ContextFrame) {
    let parent_task = sched::current().as_mut().unwrap();
    let parent_pid = parent_task.pid as usize;

    if parent_task.family.child_count == 0 {
        (*regs).set_return_error(KernelError::ECHILD);
        return;
    }

    for i in 0..parent_task.family.child_count {
        let child_pid = parent_task.family.children[i] as usize;

        let mut table = PROCESS_TABLE.lock();
        if let Some(ref mut child) = table[child_pid] {
            if child.state == ProcessState::Zombie {
                let reaped_pid = child.pid;

                let old_cr3 = child.context.cr3;
                let k_stack_bottom = child.kernel_stack_bottom;

                table[child_pid] = None;

                let last_idx = parent_task.family.child_count - 1;
                parent_task.family.children[i] = parent_task.family.children[last_idx];
                parent_task.family.child_count -= 1;

                paging::free_user_address_space(old_cr3);
                paging::free_physical_page(k_stack_bottom)
                    .consume_err("Failed to free kernel stack for reaped child process");

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

    parent_task.state = ProcessState::Waiting;
    (*regs).eip -= 2; // Re-execute the wait syscall after being woken up
}
