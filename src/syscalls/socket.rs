use crate::error::KernelError;
use crate::fs::{self, VfsNodeType};
use crate::sched::{self, ContextFrame};
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

    let task = sched::current().as_mut().unwrap();

    let fd = match task.alloc_fd() {
        Some(f) => f,
        None => {
            pr_warn!("sys_socket: no available file descriptors\n");
            (*regs).set_return_error(KernelError::EMFILE);
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

    let g_fd = match fs::alloc_open_file(node, 1) {
        Ok(g) => g,
        Err(e) => {
            ipc::close_socket(kernel_sock_idx);
            (*regs).set_return_error(e);
            return;
        }
    };

    task.fd_tbl[fd] = Some(g_fd); // Process local points to VFS global
    (*regs).set_return_value(fd as u32);
}
