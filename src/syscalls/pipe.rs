use super::args::{UserPtr, syscall_define};
use crate::{
    error::{KResult, KernelError},
    fs::{VfsNode, VfsNodeType, alloc_open_file, alloc_vfs_node, close_open_file, free_vfs_node},
    pipe::{self, PipeId},
    sched::{self, Credentials},
};

const READ_END: u32 = 0;
const WRITE_END: u32 = 1;

fn alloc_pipe_node(
    pipe_id: PipeId,
    direction: u32,
    credentials: &Credentials,
) -> KResult<*mut VfsNode> {
    let node = unsafe { alloc_vfs_node() }?;
    // SAFETY: the node was just allocated, so nothing else can reach it.
    unsafe {
        (*node).node_type = VfsNodeType::Fifo;
        (*node).inode = (pipe_id as u32) << 1 | direction;
        (*node).owner_uid = credentials.uid;
        (*node).owner_gid = credentials.gid;
        (*node).rights = 0o600;
    }
    Ok(node)
}

/// Creates a pipe and returns the global open files of its read and write
/// ends. On failure nothing stays allocated.
pub(crate) fn create_pipe_files(credentials: &Credentials) -> KResult<(usize, usize)> {
    let pipe_id = pipe::create_pipe()?;
    let close_pipe = || {
        pipe::close_read_end(pipe_id);
        pipe::close_write_end(pipe_id);
    };

    let read_node =
        alloc_pipe_node(pipe_id, READ_END, credentials).inspect_err(|_| close_pipe())?;
    let write_node = match alloc_pipe_node(pipe_id, WRITE_END, credentials) {
        Ok(node) => node,
        Err(e) => {
            // SAFETY: no open file refers to the node yet.
            unsafe { free_vfs_node(read_node) };
            close_pipe();
            return Err(e);
        }
    };

    let read_file = match alloc_open_file(read_node, 1) {
        Ok(file) => file,
        Err(e) => {
            // SAFETY: no open file refers to either node yet.
            unsafe {
                free_vfs_node(read_node);
                free_vfs_node(write_node);
            }
            close_pipe();
            return Err(e);
        }
    };
    let write_file = match alloc_open_file(write_node, 1) {
        Ok(file) => file,
        Err(e) => {
            // SAFETY: closing `read_file` closes the read end and frees
            // `read_node`, so only the write side is left to release. No open
            // file refers to `write_node`.
            unsafe {
                close_open_file(read_file);
                free_vfs_node(write_node);
            }
            pipe::close_write_end(pipe_id);
            return Err(e);
        }
    };
    Ok((read_file, write_file))
}

syscall_define! {
    pub(crate) fn syscall_pipe(pipefd: UserPtr<[i32; 2]>) {
        // Check the destination before allocating anything, so a bad pointer
        // costs nothing to undo. `write` below also checks it again.
        pipefd.validate()?;

        let task = unsafe { sched::current().as_mut() }.ok_or(KernelError::Srch)?;

        let fd0 = task.alloc_fd().ok_or(KernelError::MFile)?;
        task.fd_tbl[fd0] = Some(0); // Reserve

        let Some(fd1) = task.alloc_fd() else {
            task.fd_tbl[fd0] = None;
            return Err(KernelError::MFile);
        };
        task.fd_tbl[fd1] = Some(0); // Reserve

        let release_fds = |task: &mut sched::TaskStruct| {
            task.fd_tbl[fd0] = None;
            task.fd_tbl[fd1] = None;
        };

        let credentials = task.credentials;
        let (read_file, write_file) = match create_pipe_files(&credentials) {
            Ok(files) => files,
            Err(e) => {
                release_fds(task);
                return Err(e);
            }
        };

        task.fd_tbl[fd0] = Some(read_file);
        task.fd_tbl[fd1] = Some(write_file);

        if let Err(e) = pipefd.write(&[fd0 as i32, fd1 as i32]) {
            // The buffer was valid at the start; undo everything if it no
            // longer is.
            release_fds(task);
            unsafe {
                close_open_file(read_file);
                close_open_file(write_file);
            }
            return Err(e);
        }
        Ok(0)
    }
}
