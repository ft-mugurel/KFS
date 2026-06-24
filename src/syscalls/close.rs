use crate::error::KernelError;
use crate::fs::FileDescriptor;
use crate::ipc;
use crate::sched::{ContextFrame, CURRENT_PID, MAX_FDS_PER_PROCESS, PROCESS_TABLE};

pub(super) unsafe fn syscall_close(regs: *mut ContextFrame) {
    unsafe {
        let fd = (*regs).arg1() as usize;
        let current_pid = CURRENT_PID;
        let task = PROCESS_TABLE[current_pid].as_mut().unwrap();

        if fd >= MAX_FDS_PER_PROCESS {
            (*regs).set_return_error(KernelError::EBADF);
            return;
        }
        match task.fd_tbl[fd] {
            Some(FileDescriptor::Socket(sock_idx)) => {
                ipc::close_socket(sock_idx);
            }
            Some(FileDescriptor::TTY(_)) => {
                // Nothing to do here until we bind ttys to fds
            }
            _ => {
                (*regs).set_return_error(KernelError::EBADF);
                return;
            }
        }
        task.fd_tbl[fd] = None;
        (*regs).set_return_value(0);
    }
}
