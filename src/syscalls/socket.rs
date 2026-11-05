use super::args::syscall_define;
use crate::error::KernelError;
use crate::fs::{self, VfsNodeType};
use crate::sched;
use crate::{ipc, pr_warn};

const AF_UNIX: u32 = 1;
const SOCK_STREAM: u32 = 1;

syscall_define! {
    pub fn syscall_socket(domain: u32, sock_type: u32, protocol: u32) {
        if domain != AF_UNIX || sock_type != SOCK_STREAM || protocol != 0 {
            pr_warn!("sys_socket: unsupported domain/type/protocol");
            return Err(KernelError::AFNoSupport);
        }

        let task = unsafe { sched::current().as_mut() }.ok_or(KernelError::Srch)?;
        let Some(fd) = task.alloc_fd() else {
            pr_warn!("sys_socket: no available file descriptors");
            return Err(KernelError::MFile);
        };

        let kernel_sock_idx = ipc::create_socket().inspect_err(|_| {
            pr_warn!("sys_socket: failed to create socket");
        })?;

        let node = match unsafe { fs::alloc_vfs_node() } {
            Ok(node) => node,
            Err(e) => {
                pr_warn!("sys_socket: failed to allocate VFS node");
                ipc::close_socket(kernel_sock_idx);
                return Err(e);
            }
        };

        let socket = unsafe { &mut *node };
        socket.node_type = VfsNodeType::Socket;
        socket.inode = kernel_sock_idx as u32;
        socket.next_of_kin = core::ptr::null_mut();
        socket.children = core::ptr::null_mut();
        socket.father = core::ptr::null_mut();
        socket.master = core::ptr::null_mut();
        // Default permissions for sockets
        socket.rights = 0o666;
        socket.links = 1;
        // Sockets don't have a static file size
        socket.size = 0;
        // TODO: this is for debug, but not sure what else to do yet.
        let name = b"anon_socket\0";
        socket.name[..name.len()].copy_from_slice(name);

        let g_fd = match fs::alloc_open_file(node, 1) {
            Ok(g_fd) => g_fd,
            Err(e) => {
                ipc::close_socket(kernel_sock_idx);
                unsafe { fs::free_vfs_node(node) };
                return Err(e);
            }
        };

        task.fd_tbl[fd] = Some(g_fd); // Process local points to VFS global
        Ok(fd as u32)
    }
}
