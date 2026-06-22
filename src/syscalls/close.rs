use crate::sched::scheduler::{CURRENT_PID, PROCESS_TABLE};
use crate::sched::task::ContextFrame;

pub(super) unsafe fn syscall_close(regs: *mut ContextFrame) {
    unsafe {
        let fd = (*regs).arg1() as usize;
        let current_pid = CURRENT_PID;
        let task = PROCESS_TABLE[current_pid].as_mut().unwrap();

        if fd >= crate::sched::task::MAX_FDS_PER_PROCESS {
            (*regs).set_return_value(!0u32); // EBADF
            return;
        }
        match task.fd_tbl[fd] {
            Some(crate::fs::vfs::FileDescriptor::Socket(sock_idx)) => {
                crate::ipc::close_socket(sock_idx);
            }
            Some(crate::fs::vfs::FileDescriptor::TTY(_)) => {
                // Nothing to do here until we bind ttys to fds
            }
            _ => {
                (*regs).set_return_value(!0u32); // EBADF
                return;
            }
        }
        task.fd_tbl[fd] = None;
        (*regs).set_return_value(0);
    }
}
