use crate::error::KernelError;
use crate::fs::{VfsNodeType, OPEN_FILE_TABLE};
use crate::ipc::{self};
use crate::sched::{self, ContextFrame, MAX_FDS_PER_PROCESS};
use crate::{pr_debug, pr_warn};

pub unsafe fn syscall_read(regs: *mut ContextFrame) {
    let local_fd = (*regs).arg1() as usize;
    let buf_ptr = (*regs).arg2() as *mut u8;
    let count = (*regs).arg3() as usize;

    if local_fd >= MAX_FDS_PER_PROCESS {
        (*regs).set_return_error(KernelError::EBADF);
        return;
    }

    let fd_tbl = &mut sched::current().as_mut().unwrap().fd_tbl;

    let global_fd = match fd_tbl[local_fd] {
        Some(idx) => idx,
        _ => {
            pr_warn!("sys_read: invalid local file descriptor {}\n", local_fd,);
            (*regs).set_return_error(KernelError::EBADF);
            return;
        }
    };

    let open_file = match &mut OPEN_FILE_TABLE[global_fd] {
        Some(file) => file,
        None => {
            pr_warn!("sys_read: invalid global file descriptor {}\n", global_fd);
            (*regs).set_return_error(KernelError::EBADF);
            return;
        }
    };

    let user_buffer = core::slice::from_raw_parts_mut(buf_ptr, count);
    let node = open_file.node;
    match (*node).node_type {
        VfsNodeType::File | VfsNodeType::BlockDevice => {
            match (*node).read(user_buffer, open_file.offset) {
                Ok(read) => {
                    open_file.offset += read as u32;
                    (*regs).set_return_value(read as u32);
                }
                Err(e) => {
                    pr_warn!("sys_read: error reading from file: {:?}\n", e);
                    (*regs).set_return_error(KernelError::EIO);
                }
            }
        }
        VfsNodeType::Socket => {
            let sock_idx = (*node).inode as usize;
            pr_debug!("Reading from socket {}\n", sock_idx);
            match ipc::read_socket(sock_idx, user_buffer) {
                Ok(read) => (*regs).set_return_value(read as u32),
                Err(e) => {
                    pr_warn!("sys_read: error reading from socket: {:?}\n", e);
                    (*regs).set_return_error(e)
                }
            }
        }
        VfsNodeType::Directory => {
            pr_warn!("sys_read: attempted to read from a directory\n");
            (*regs).set_return_error(KernelError::EISDIR);
            return;
        }
        _ => {
            pr_warn!("sys_read: invalid node type\n");
            (*regs).set_return_error(KernelError::EINVAL);
            return;
        }
    };
}

pub unsafe fn syscall_write(regs: *mut ContextFrame) {
    let local_fd = (*regs).arg1() as usize;
    let buf_ptr = (*regs).arg2() as *const u8;
    let count = (*regs).arg3() as usize;

    pr_debug!("sys_write called with fd={}, count={}\n", local_fd, count);

    if local_fd >= MAX_FDS_PER_PROCESS {
        (*regs).set_return_error(KernelError::EBADF);
        return;
    }

    let fd_tbl = &mut sched::current().as_mut().unwrap().fd_tbl;

    if local_fd == 0 || local_fd == 1 || local_fd == 2 {
        if count > 1024 {
            pr_warn!("sys_write: write size {} exceeds limit\n", count);
            (*regs).set_return_error(KernelError::EFBIG);
            return;
        }
        let msg = core::slice::from_raw_parts(buf_ptr, count);
        if let Ok(s) = core::str::from_utf8(msg) {
            pr_debug!("Writing to console: {:?}\n", s);
        }
        (*regs).set_return_value(count as u32);
        return;
    }

    let global_fd = match fd_tbl[local_fd] {
        Some(idx) => idx,
        _ => {
            pr_warn!("sys_write: invalid local file descriptor {}\n", local_fd);
            (*regs).set_return_error(KernelError::EBADF);
            return;
        }
    };

    let open_file = match &mut OPEN_FILE_TABLE[global_fd] {
        Some(file) => file,
        None => {
            pr_warn!("sys_write: invalid global file descriptor {}\n", global_fd);
            (*regs).set_return_error(KernelError::EBADF);
            return;
        }
    };

    let user_buffer = core::slice::from_raw_parts(buf_ptr, count);
    let node = open_file.node;

    match (*node).node_type {
        VfsNodeType::File | VfsNodeType::BlockDevice => {
            match (*node).write(user_buffer, open_file.offset) {
                Ok(written) => {
                    open_file.offset += written as u32;
                    (*regs).set_return_value(written as u32);
                }
                Err(e) => {
                    pr_warn!("sys_write: error writing to file: {:?}\n", e);
                    (*regs).set_return_error(KernelError::EIO);
                }
            }
        }
        VfsNodeType::Socket => {
            let sock_idx = (*node).inode as usize;
            pr_debug!("Writing to socket {}\n", sock_idx);
            match ipc::write_socket(sock_idx, user_buffer) {
                Ok(written) => (*regs).set_return_value(written as u32),
                Err(e) => {
                    pr_warn!("sys_write: error writing to socket: {:?}\n", e);
                    (*regs).set_return_error(e);
                }
            }
        }
        _ => {
            pr_warn!("sys_write: invalid node type for writing\n");
            (*regs).set_return_error(KernelError::EINVAL);
            return;
        }
    };
}
