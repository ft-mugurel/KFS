use crate::error::KernelError;
use crate::fs::{self, VfsNodeType};
use crate::ipc::{self};
use crate::sched::{self, ContextFrame, MAX_FDS_PER_PROCESS};
use crate::{pr_debug, pr_warn};
use crate::security::{self, Decision, Operation};

unsafe fn valid_user_buffer(ptr: u32, len: usize) -> bool {
    if len == 0 {
        return true;
    }

    let start = ptr as u64;
    let end = match start.checked_add(len as u64) {
        Some(end) if end <= (u32::MAX as u64) + 1 => end,
        _ => return false,
    };

    let task = match sched::current().as_ref() {
        Some(task) => task,
        None => return false,
    };
    let mem = &task.memory;

    let in_region = |base: u32, size: u32| {
        let region_start = base as u64;
        let region_end = region_start + size as u64;
        start >= region_start && end <= region_end
    };

    in_region(mem.code_base, mem.code_size)
        || in_region(mem.data_base, mem.data_size)
        || in_region(mem.bss_base, mem.bss_size)
        || (start >= mem.stack_limit as u64 && end <= mem.stack_base as u64)
        || (start >= mem.heap_base as u64 && end <= mem.heap_brk as u64)
        || mem
            .vmas
            .iter()
            .any(|vma| vma.used && in_region(vma.base, vma.size))
}

pub unsafe fn syscall_read(regs: *mut ContextFrame) {
    let local_fd = (*regs).arg1() as usize;
    let buf_ptr = (*regs).arg2() as *mut u8;
    let count = (*regs).arg3() as usize;

    if !valid_user_buffer(buf_ptr as u32, count) {
        (*regs).set_return_error(KernelError::EFAULT);
        return;
    }

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

    let open_file = match fs::get_open_file(global_fd) {
        Some(file) => file,
        None => {
            pr_warn!("sys_read: invalid global file descriptor {}\n", global_fd);
            (*regs).set_return_error(KernelError::EBADF);
            return;
        }
    };

    let user_buffer = core::slice::from_raw_parts_mut(buf_ptr, count);
    let node = open_file.node;
    let credentials = &sched::current().as_ref().unwrap().credentials;
    match (*node).node_type {
        VfsNodeType::File | VfsNodeType::BlockDevice | VfsNodeType::CharDevice => {
            if security::check(
                credentials,
                &security::object_for_node(node),
                Operation::Read,
            ) == Decision::Deny
            {
                (*regs).set_return_error(KernelError::EACCES);
                return;
            }
            match (*node).read(user_buffer, open_file.offset) {
                Ok(read) => {
                    fs::update_open_file_offset(global_fd, read as u32);
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

    if !valid_user_buffer(buf_ptr as u32, count) {
        (*regs).set_return_error(KernelError::EFAULT);
        return;
    }

    if local_fd >= MAX_FDS_PER_PROCESS {
        (*regs).set_return_error(KernelError::EBADF);
        return;
    }

    let fd_tbl = &mut sched::current().as_mut().unwrap().fd_tbl;

    let global_fd = match fd_tbl[local_fd] {
        Some(idx) => idx,
        _ => {
            pr_warn!("sys_write: invalid local file descriptor {}\n", local_fd);
            (*regs).set_return_error(KernelError::EBADF);
            return;
        }
    };

    let open_file = match fs::get_open_file(global_fd) {
        Some(file) => file,
        None => {
            pr_warn!("sys_write: invalid global file descriptor {}\n", global_fd);
            (*regs).set_return_error(KernelError::EBADF);
            return;
        }
    };

    let user_buffer = core::slice::from_raw_parts(buf_ptr, count);
    let node = open_file.node;
    let credentials = &sched::current().as_ref().unwrap().credentials;

    match (*node).node_type {
        VfsNodeType::File | VfsNodeType::BlockDevice | VfsNodeType::CharDevice => {
            if security::check(
                credentials,
                &security::object_for_node(node),
                Operation::Write,
            ) == Decision::Deny
            {
                (*regs).set_return_error(KernelError::EACCES);
                return;
            }
            match (*node).write(user_buffer, open_file.offset) {
                Ok(written) => {
                    fs::update_open_file_offset(global_fd, written as u32);
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
