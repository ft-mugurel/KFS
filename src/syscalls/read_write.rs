use super::args::{UserAddr, UserSlice, syscall_define};
use crate::{
    error::KernelError,
    fs::{self, VfsNodeType},
    ipc::{self},
    sched::{self, MAX_FDS_PER_PROCESS},
    security::{self, Decision, Operation},
    {pr_debug, pr_warn},
};

pub(super) unsafe fn valid_user_buffer(ptr: u32, len: usize) -> bool {
    if len == 0 {
        return true;
    }

    let start = ptr as u64;
    let end = match start.checked_add(len as u64) {
        Some(end) if end <= (u32::MAX as u64) + 1 => end,
        _ => return false,
    };

    let task = match unsafe { sched::current().as_ref() } {
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

syscall_define! {
    pub fn syscall_read(local_fd: usize, buf: UserAddr, count: usize) {
        let mut buffer = UserSlice::new(buf, count)?;

        if local_fd >= MAX_FDS_PER_PROCESS {
            return Err(KernelError::BadF);
        }

        // Copy what's needed out of the task so no reference to it is alive
        // while the VFS runs (a tty read may mark the task Waiting).
        let (local_entry, credentials) = {
            let task = unsafe { sched::current().as_ref() }.ok_or(KernelError::Srch)?;
            (task.fd_tbl[local_fd], task.credentials)
        };

        let global_fd = local_entry.ok_or_else(|| {
            pr_warn!("sys_read: invalid local file descriptor {}", local_fd);
            KernelError::BadF
        })?;
        let open_file = fs::get_open_file(global_fd).ok_or_else(|| {
            pr_warn!("sys_read: invalid global file descriptor {}", global_fd);
            KernelError::BadF
        })?;
        let node = open_file.node;

        match unsafe { (*node).node_type } {
            VfsNodeType::File
            | VfsNodeType::BlockDevice
            | VfsNodeType::CharDevice
            | VfsNodeType::Fifo => {
                let object = unsafe { security::object_for_node(node) };
                if security::check(&credentials, &object, Operation::Read) == Decision::Deny {
                    return Err(KernelError::Acces);
                }
                match unsafe { (*node).read(buffer.as_bytes_mut(), open_file.offset) } {
                    Ok(read) => {
                        fs::update_open_file_offset(global_fd, read as u32);
                        Ok(read as u32)
                    }
                    // Waiting for input: retried when the task is woken.
                    Err(KernelError::Again) => Err(KernelError::Restart),
                    Err(e) => {
                        pr_warn!("sys_read: error reading from file: {:?}", e);
                        Err(e)
                    }
                }
            }
            VfsNodeType::Socket => {
                let sock_idx = unsafe { (*node).inode } as usize;
                pr_debug!("Reading from socket {}", sock_idx);
                ipc::read_socket(sock_idx, buffer.as_bytes_mut())
                    .map(|read| read as u32)
                    .inspect_err(|e| pr_warn!("sys_read: error reading from socket: {:?}", e))
            }
            VfsNodeType::Directory => {
                pr_warn!("sys_read: attempted to read from a directory");
                Err(KernelError::IsDir)
            }
            _ => {
                pr_warn!("sys_read: invalid node type");
                Err(KernelError::Inval)
            }
        }
    }
}

syscall_define! {
    pub fn syscall_write(local_fd: usize, buf: UserAddr, count: usize) {
        let buffer = UserSlice::new(buf, count)?;

        if local_fd >= MAX_FDS_PER_PROCESS {
            return Err(KernelError::BadF);
        }

        // Copy what's needed out of the task so no reference to it is alive
        // while the VFS runs (it may block, or take PROCESS_TABLE).
        let (local_entry, credentials) = {
            let task = unsafe { sched::current().as_ref() }.ok_or(KernelError::Srch)?;
            (task.fd_tbl[local_fd], task.credentials)
        };

        let global_fd = local_entry.ok_or_else(|| {
            pr_warn!("sys_write: invalid local file descriptor {}", local_fd);
            KernelError::BadF
        })?;
        let open_file = fs::get_open_file(global_fd).ok_or_else(|| {
            pr_warn!("sys_write: invalid global file descriptor {}", global_fd);
            KernelError::BadF
        })?;
        let node = open_file.node;

        match unsafe { (*node).node_type } {
            VfsNodeType::File
            | VfsNodeType::BlockDevice
            | VfsNodeType::CharDevice
            | VfsNodeType::Fifo => {
                let object = unsafe { security::object_for_node(node) };
                if security::check(&credentials, &object, Operation::Write) == Decision::Deny {
                    return Err(KernelError::Acces);
                }
                match unsafe { (*node).write(buffer.as_bytes(), open_file.offset) } {
                    Ok(written) => {
                        fs::update_open_file_offset(global_fd, written as u32);
                        Ok(written as u32)
                    }
                    // Waiting for buffer space: retried when the task is woken.
                    Err(KernelError::Again) => Err(KernelError::Restart),
                    Err(e) => {
                        pr_warn!("sys_write: error writing to file: {:?}", e);
                        Err(e)
                    }
                }
            }
            VfsNodeType::Socket => {
                let sock_idx = unsafe { (*node).inode } as usize;
                pr_debug!("Writing to socket {}", sock_idx);
                ipc::write_socket(sock_idx, buffer.as_bytes())
                    .map(|written| written as u32)
                    .inspect_err(|e| pr_warn!("sys_write: error writing to socket: {:?}", e))
            }
            _ => {
                pr_warn!("sys_write: invalid node type for writing");
                Err(KernelError::Inval)
            }
        }
    }
}
