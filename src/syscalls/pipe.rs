use crate::{
    error::KernelError,
    fs::{VfsNodeType, alloc_open_file, alloc_vfs_node, close_open_file},
    pipe,
    sched::{self, ContextFrame},
};

pub(crate) unsafe fn syscall_pipe(regs: *mut ContextFrame) {
    let pipefd_ptr = (*regs).ebx as *mut i32;

    if pipefd_ptr.is_null() || pipefd_ptr as usize > 0xC000_0000 - 8 {
        (*regs).set_return_error(KernelError::EFAULT);
        return;
    }

    let task = sched::current().as_mut().unwrap();

    let fd0 = match task.alloc_fd() {
        Some(fd) => fd,
        None => {
            (*regs).set_return_error(KernelError::EMFILE);
            return;
        }
    };
    task.fd_tbl[fd0] = Some(0); // Reserve

    let fd1 = match task.alloc_fd() {
        Some(fd) => fd,
        None => {
            task.fd_tbl[fd0] = None;
            (*regs).set_return_error(KernelError::EMFILE);
            return;
        }
    };
    task.fd_tbl[fd1] = Some(0); // Reserve

    let pipe_id = match pipe::create_pipe() {
        Ok(id) => id,
        Err(e) => {
            task.fd_tbl[fd0] = None;
            task.fd_tbl[fd1] = None;
            (*regs).set_return_error(e);
            return;
        }
    };

    let read_node = match alloc_vfs_node() {
        Ok(node) => node,
        Err(e) => {
            task.fd_tbl[fd0] = None;
            task.fd_tbl[fd1] = None;
            pipe::close_read_end(pipe_id);
            pipe::close_write_end(pipe_id);
            (*regs).set_return_error(e);
            return;
        }
    };

    let write_node = match alloc_vfs_node() {
        Ok(node) => node,
        Err(e) => {
            // we should probably free read_node but we don't have a free function right now
            task.fd_tbl[fd0] = None;
            task.fd_tbl[fd1] = None;
            pipe::close_read_end(pipe_id);
            pipe::close_write_end(pipe_id);
            (*regs).set_return_error(e);
            return;
        }
    };

    (*read_node).node_type = VfsNodeType::Fifo;
    (*read_node).inode = (pipe_id as u32) << 1 | 0; // direction 0 = read
    (*read_node).owner_uid = task.credentials.uid;
    (*read_node).owner_gid = task.credentials.gid;
    (*read_node).rights = 0o600;

    (*write_node).node_type = VfsNodeType::Fifo;
    (*write_node).inode = (pipe_id as u32) << 1 | 1; // direction 1 = write
    (*write_node).owner_uid = task.credentials.uid;
    (*write_node).owner_gid = task.credentials.gid;
    (*write_node).rights = 0o600;

    let global_fd0 = match alloc_open_file(read_node, 1) {
        Ok(gfd) => gfd,
        Err(e) => {
            task.fd_tbl[fd0] = None;
            task.fd_tbl[fd1] = None;
            pipe::close_read_end(pipe_id);
            pipe::close_write_end(pipe_id);
            (*regs).set_return_error(e);
            return;
        }
    };

    let global_fd1 = match alloc_open_file(write_node, 1) {
        Ok(gfd) => gfd,
        Err(e) => {
            task.fd_tbl[fd0] = None;
            task.fd_tbl[fd1] = None;
            close_open_file(global_fd0);
            pipe::close_write_end(pipe_id);
            (*regs).set_return_error(e);
            return;
        }
    };

    task.fd_tbl[fd0] = Some(global_fd0);
    task.fd_tbl[fd1] = Some(global_fd1);

    core::ptr::write(pipefd_ptr, fd0 as i32);
    core::ptr::write(pipefd_ptr.add(1), fd1 as i32);

    (*regs).set_return_value(0);
}
