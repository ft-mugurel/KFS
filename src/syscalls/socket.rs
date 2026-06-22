use crate::sched::scheduler::{CURRENT_PID, PROCESS_TABLE};
use crate::sched::task::ContextFrame;

pub(super) unsafe fn syscall_socket(regs: *mut ContextFrame) {
    unsafe {
        let current_pid = CURRENT_PID;
        let task = PROCESS_TABLE[current_pid].as_mut().unwrap();

        let mut local_fd = None;
        for i in 3..crate::sched::task::MAX_FDS_PER_PROCESS {
            if task.fd_tbl[i].is_none() {
                crate::pr_warn!(
                    "Allocating fd {} for new socket in process {}\n",
                    i,
                    current_pid
                );
                local_fd = Some(i);
                break;
            } else {
                crate::pr_warn!("fd {} is already in use for process {}\n", i, current_pid);
            }
        }
        if let Some(fd) = local_fd {
            if let Some(kernel_sock_idx) = crate::ipc::create_socket() {
                task.fd_tbl[fd] = Some(crate::fs::vfs::FileDescriptor::Socket(kernel_sock_idx));

                (*regs).set_return_value(fd as u32);
            } else {
                crate::pr_warn!("Failed to create socket: no available kernel socket slots\n");
                (*regs).set_return_value(!0u32);
            }
        } else {
            crate::pr_warn!(
                "Failed to create socket: no available fd slots for process {}\n",
                current_pid
            );
            (*regs).set_return_value(!0u32);
        }
    }
}
