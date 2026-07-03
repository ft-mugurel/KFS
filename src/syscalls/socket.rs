use crate::error::KernelError;
use crate::fs::{self, OpenFile, VfsNodeType, MAX_OPEN_FILES, OPEN_FILE_TABLE};
use crate::sched::{ContextFrame, CURRENT_PID, MAX_FDS_PER_PROCESS, PROCESS_TABLE};
use crate::{ipc, pr_warn};

const AF_UNIX: u32 = 1;
const SOCK_STREAM: u32 = 1;

pub unsafe fn syscall_socket(regs: *mut ContextFrame) {
    let domain = (*regs).arg1();
    let sock_type = (*regs).arg2();
    let protocol = (*regs).arg3();

    if domain != AF_UNIX || sock_type != SOCK_STREAM || protocol != 0 {
        pr_warn!("sys_socket: unsupported domain/type/protocol\n");
        (*regs).set_return_error(KernelError::EAFNOSUPPORT);
        return;
    }

    let current_pid = CURRENT_PID;
    let task = PROCESS_TABLE[current_pid].as_mut().unwrap();

    let mut local_fd = None;
    for i in 3..MAX_FDS_PER_PROCESS {
        if task.fd_tbl[i].is_none() {
            local_fd = Some(i);
            break;
        }
    }
    let fd = match local_fd {
        Some(f) => f,
        None => {
            pr_warn!(
                "sys_socket: no available file descriptors for process {}\n",
                current_pid
            );
            (*regs).set_return_error(KernelError::EMFILE);
            return;
        }
    };

    let mut global_fd = None;
    for i in 0..MAX_OPEN_FILES {
        if crate::fs::OPEN_FILE_TABLE[i].is_none() {
            global_fd = Some(i);
            break;
        }
    }
    let g_fd = match global_fd {
        Some(g) => g,
        None => {
            pr_warn!("sys_socket: no available global file descriptors\n");
            (*regs).set_return_error(KernelError::ENFILE);
            return;
        }
    };

    let kernel_sock_idx = match ipc::create_socket() {
        Ok(idx) => idx,
        Err(e) => {
            pr_warn!("sys_socket: failed to create socket\n");
            (*regs).set_return_error(e);
            return;
        }
    };

    let node = match fs::alloc_vfs_node() {
        Ok(n) => n,
        Err(e) => {
            pr_warn!("sys_socket: failed to allocate VFS node\n");
            ipc::close_socket(kernel_sock_idx);
            (*regs).set_return_error(e);
            return;
        }
    };

    // Initialize the VFS node
    (*node).node_type = VfsNodeType::Socket;
    (*node).inode = kernel_sock_idx as u32;
    (*node).next_of_kin = core::ptr::null_mut();
    (*node).children = core::ptr::null_mut();
    (*node).father = core::ptr::null_mut();
    (*node).master = core::ptr::null_mut();
    // Default permissions for sockets
    (*node).rights = 0o666;
    (*node).links = 1;
    // Sockets don't have a static file size
    (*node).size = 0;

    // TODO: this is for debug, but not sure what else to do yet.
    let name = b"anon_socket\0";
    core::ptr::copy_nonoverlapping(name.as_ptr(), (*node).name.as_mut_ptr(), name.len());

    OPEN_FILE_TABLE[g_fd] = Some(OpenFile {
        node,
        offset: 0, // (read/write)_socket will ignore this
        ref_count: 1,
    });

    task.fd_tbl[fd] = Some(g_fd); // Process local points to VFS global
    (*regs).set_return_value(fd as u32);
}
