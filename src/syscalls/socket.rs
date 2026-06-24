use crate::error::KernelError;
use crate::fs::FileDescriptor;
use crate::sched::{ContextFrame, CURRENT_PID, MAX_FDS_PER_PROCESS, PROCESS_TABLE};
use crate::{ipc, pr_debug, pr_warn};

pub(super) unsafe fn syscall_socket(regs: *mut ContextFrame) {
    unsafe {
        let current_pid = CURRENT_PID;
        let task = PROCESS_TABLE[current_pid].as_mut().unwrap();

        let mut local_fd = None;
        for i in 3..MAX_FDS_PER_PROCESS {
            if task.fd_tbl[i].is_none() {
                pr_debug!(
                    "Allocating fd {} for new socket in process {}\n",
                    i,
                    current_pid
                );
                local_fd = Some(i);
                break;
            } else {
                pr_warn!("fd {} is already in use for process {}\n", i, current_pid);
            }
        }
        if let Some(fd) = local_fd {
            if let Some(kernel_sock_idx) = ipc::create_socket() {
                task.fd_tbl[fd] = Some(FileDescriptor::Socket(kernel_sock_idx));

                (*regs).set_return_value(fd as u32);
            } else {
                pr_warn!("Failed to create socket: no available kernel socket slots\n");
                (*regs).set_return_error(KernelError::ENFILE);
            }
        } else {
            pr_warn!(
                "Failed to create socket: no available fd slots for process {}\n",
                current_pid
            );
            (*regs).set_return_error(KernelError::EMFILE);
        }
    }
}
