use crate::error::KernelError;
use crate::fs::{self, OpenFile, VfsNodeType, MAX_OPEN_FILES, OPEN_FILE_TABLE};
use crate::ipc;
use crate::sched::{ContextFrame, CURRENT_PID, MAX_FDS_PER_PROCESS, PROCESS_TABLE};
use crate::utils;

pub unsafe fn syscall_open(regs: *mut ContextFrame) {
    let path_ptr = (*regs).arg1() as *const u8;
    // let flags = (*regs).arg2();
    // let mode = (*regs).arg3();

    let path_str = utils::c_str_to_rust(path_ptr);

    let task = PROCESS_TABLE[CURRENT_PID].as_mut().unwrap();

    let node = match fs::resolve_path(path_str, task.cwd) {
        Ok(n) => n,
        Err(_) => {
            (*regs).set_return_error(KernelError::ENOENT);
            return;
        }
    };

    let mut local_fd = None;
    for i in 3..MAX_FDS_PER_PROCESS {
        if task.fd_tbl[i].is_none() {
            local_fd = Some(i);
            break;
        }
    }

    let l_fd = match local_fd {
        Some(f) => f,
        None => {
            (*regs).set_return_error(KernelError::EMFILE);
            return;
        }
    };

    let mut global_fd = None;
    for i in 0..MAX_OPEN_FILES {
        if OPEN_FILE_TABLE[i].is_none() {
            global_fd = Some(i);
            break;
        }
    }
    let g_fd = match global_fd {
        Some(g) => g,
        None => {
            (*regs).set_return_error(KernelError::ENFILE);
            return;
        }
    };

    OPEN_FILE_TABLE[g_fd] = Some(OpenFile { node, offset: 0, ref_count: 1 });
    task.fd_tbl[l_fd] = Some(g_fd);

    (*regs).set_return_value(l_fd as u32);
}

pub unsafe fn syscall_close(regs: *mut ContextFrame) {
    let local_fd = (*regs).arg1() as usize;

    if local_fd >= MAX_FDS_PER_PROCESS {
        (*regs).set_return_error(KernelError::EBADF);
        return;
    }

    let task = PROCESS_TABLE[CURRENT_PID].as_mut().unwrap();

    let global_fd = match task.fd_tbl[local_fd] {
        Some(idx) => idx,
        _ => {
            (*regs).set_return_error(KernelError::EBADF);
            return;
        }
    };

    task.fd_tbl[local_fd] = None;

    if let Some(open_file) = &mut OPEN_FILE_TABLE[global_fd] {
        if open_file.ref_count > 0 {
            open_file.ref_count -= 1;
        }

        if open_file.ref_count == 0 {
            let node = open_file.node;
            if (*node).node_type == VfsNodeType::Socket {
                let sock_idx = (*node).inode as usize;
                ipc::close_socket(sock_idx);
            }

            OPEN_FILE_TABLE[global_fd] = None;
        }
    }

    (*regs).set_return_value(0);
}
