use crate::error::KernelError;
use crate::fs::{self, VfsNodeType};
use crate::sched::{self, ContextFrame, MAX_FDS_PER_PROCESS};
use crate::utils;
use crate::security::{self, Decision, Operation};

const O_CREAT: u32 = 0x40;
const O_TRUNC: u32 = 0x200;

fn split_parent_path(path: &str) -> Result<(&str, &str), KernelError> {
    if path.is_empty() || path == "/" {
        return Err(KernelError::E2BIG);
    }

    if let Some((parent, name)) = path.rsplit_once('/') {
        let parent_path = if parent.is_empty() { "/" } else { parent };
        if name.is_empty() {
            return Err(KernelError::ETXTBSY);
        }
        Ok((parent_path, name))
    } else {
        Ok(("", path))
    }
}

pub unsafe fn syscall_open(regs: *mut ContextFrame) {
    let path_ptr = (*regs).arg1() as *const u8;
    let flags = (*regs).arg2();
    let mode = (*regs).arg3();

    let path_str = utils::c_str_to_rust(path_ptr);

    let task = sched::current().as_mut().unwrap();
    let cwd = task.cwd;

    let node = match fs::resolve_path(path_str, cwd) {
        Ok(n) => n,
        Err(err) => {
            if err != KernelError::ENOENT || (flags & O_CREAT) == 0 {
                (*regs).set_return_error(err);
                return;
            }

            let (parent_path, name) = match split_parent_path(path_str) {
                Ok(parts) => parts,
                Err(split_err) => {
                    (*regs).set_return_error(split_err);
                    return;
                }
            };

            let parent = match fs::resolve_path(parent_path, cwd) {
                Ok(parent_node) => parent_node,
                Err(parent_err) => {
                    (*regs).set_return_error(parent_err);
                    return;
                }
            };

            if (*parent).node_type != VfsNodeType::Directory {
                (*regs).set_return_error(KernelError::ENOTDIR);
                return;
            }

            if security::check(
                &task.credentials,
                &security::object_for_node(parent),
                Operation::Create,
            ) == Decision::Deny
            {
                (*regs).set_return_error(KernelError::EACCES);
                return;
            }

            match fs::create_child_node(parent, name, VfsNodeType::File, (mode & 0x0FFF) as u16) {
                Ok(new_node) => new_node,
                Err(create_err) => {
                    (*regs).set_return_error(create_err);
                    return;
                }
            }
        }
    };

    let requested = match flags & 0x3 {
        0 => 0o4,
        1 => 0o2,
        2 => 0o6,
        _ => {
            (*regs).set_return_error(KernelError::EINVAL);
            return;
        }
    };
    let operation = match requested {
        0o4 => Operation::Read,
        0o2 => Operation::Write,
        0o6 => Operation::ReadWrite,
        _ => unreachable!(),
    };
    if security::check(
        &task.credentials,
        &security::object_for_node(node),
        operation,
    ) == Decision::Deny
    {
        (*regs).set_return_error(KernelError::EACCES);
        return;
    }

    if (flags & O_TRUNC) != 0 {
        if (*node).node_type == VfsNodeType::Directory {
            (*regs).set_return_error(KernelError::EISDIR);
            return;
        }

        if security::check(
            &task.credentials,
            &security::object_for_node(node),
            Operation::Truncate,
        ) == Decision::Deny
        {
            (*regs).set_return_error(KernelError::EACCES);
            return;
        }

        if let Err(err) = (*node).truncate() {
            (*regs).set_return_error(err);
            return;
        }
    }

    let l_fd = match task.alloc_fd() {
        Some(f) => f,
        None => {
            (*regs).set_return_error(KernelError::EMFILE);
            return;
        }
    };

    let g_fd = match fs::alloc_open_file(node, 1) {
        Ok(g) => g,
        Err(e) => {
            (*regs).set_return_error(e);
            return;
        }
    };

    task.fd_tbl[l_fd] = Some(g_fd);
    (*regs).set_return_value(l_fd as u32);
}

pub unsafe fn syscall_close(regs: *mut ContextFrame) {
    let local_fd = (*regs).arg1() as usize;

    if local_fd >= MAX_FDS_PER_PROCESS {
        (*regs).set_return_error(KernelError::EBADF);
        return;
    }

    let fd_tbl = &mut sched::current().as_mut().unwrap().fd_tbl;

    let global_fd = match fd_tbl[local_fd] {
        Some(idx) => idx,
        _ => {
            (*regs).set_return_error(KernelError::EBADF);
            return;
        }
    };

    fd_tbl[local_fd] = None;
    fs::close_open_file(global_fd);
    (*regs).set_return_value(0);
}
