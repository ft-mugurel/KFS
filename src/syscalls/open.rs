use kfs_abi::{O_CREAT, O_TRUNC};

use super::args::{Caller, PATH_MAX, UserPtr, caller, syscall_define};
use crate::{
    error::{KResult, KernelError},
    fs::{self, VfsNode, VfsNodeType},
    sched::{self, MAX_FDS_PER_PROCESS},
    security::{self, Decision, Operation},
};

fn split_parent_path(path: &str) -> Result<(&str, &str), KernelError> {
    if path.is_empty() || path == "/" {
        return Err(KernelError::TooBig);
    }

    if let Some((parent, name)) = path.rsplit_once('/') {
        let parent_path = if parent.is_empty() { "/" } else { parent };
        if name.is_empty() {
            return Err(KernelError::TxtBsy);
        }
        Ok((parent_path, name))
    } else {
        Ok(("", path))
    }
}

unsafe fn create_file(path: &str, caller: &Caller, mode: u32) -> KResult<*mut VfsNode> {
    let (parent_path, name) = split_parent_path(path)?;
    let parent = unsafe { fs::resolve_path(parent_path, caller.cwd) }?;
    if unsafe { (*parent).node_type } != VfsNodeType::Directory {
        return Err(KernelError::NotDir);
    }
    let object = unsafe { security::object_for_node(parent) };
    if security::check(&caller.credentials, &object, Operation::Create) == Decision::Deny {
        return Err(KernelError::Acces);
    }
    unsafe { fs::create_child_node(parent, name, VfsNodeType::File, (mode & 0x0FFF) as u16) }
}

syscall_define! {
    pub fn syscall_open(path: UserPtr<u8>, flags: u32, mode: u32) {
        let path = path.read_cstr(PATH_MAX)?;
        // Check the access mode first, so an invalid one creates nothing.
        let operation = match flags & 0x3 {
            0 => Operation::Read,
            1 => Operation::Write,
            2 => Operation::ReadWrite,
            _ => return Err(KernelError::Inval),
        };
        let caller = caller()?;
        let _pin = fs::pin_nodes();

        let node = match unsafe { fs::resolve_path(path, caller.cwd) } {
            Ok(node) => node,
            Err(KernelError::NoEnt) if (flags & O_CREAT) != 0 => unsafe { create_file(path, &caller, mode) }?,
            Err(err) => return Err(err),
        };

        let object = unsafe { security::object_for_node(node) };
        if security::check(&caller.credentials, &object, operation) == Decision::Deny {
            return Err(KernelError::Acces);
        }

        if (flags & O_TRUNC) != 0 {
            if unsafe { (*node).node_type } == VfsNodeType::Directory {
                return Err(KernelError::IsDir);
            }
            if security::check(&caller.credentials, &object, Operation::Truncate) == Decision::Deny {
                return Err(KernelError::Acces);
            }
            unsafe { (*node).truncate() }?;
        }

        let task = unsafe { sched::current().as_mut() }.ok_or(KernelError::Srch)?;
        let l_fd = task.alloc_fd().ok_or(KernelError::MFile)?;
        let g_fd = fs::alloc_open_file(node, 1)?;
        unsafe { fs::vfs_ref_get(node) };
        task.fd_tbl[l_fd] = Some(g_fd);
        Ok(l_fd as u32)
    }
}

syscall_define! {
    pub fn syscall_close(local_fd: usize) {
        if local_fd >= MAX_FDS_PER_PROCESS {
            return Err(KernelError::BadF);
        }
        let fd_tbl = &mut unsafe { sched::current().as_mut() }.ok_or(KernelError::Srch)?.fd_tbl;
        let global_fd = fd_tbl[local_fd].take().ok_or(KernelError::BadF)?;

        let node = fs::get_open_file(global_fd).map_or(core::ptr::null_mut(), |f| f.node);
        // Before closing: the last close can free the node.
        unsafe { fs::vfs_ref_put(node) };
        unsafe { fs::close_open_file(global_fd) };
        Ok(0)
    }
}

syscall_define! {
    pub fn syscall_dup2(old_fd: usize, new_fd: usize) {
        if old_fd >= MAX_FDS_PER_PROCESS || new_fd >= MAX_FDS_PER_PROCESS {
            return Err(KernelError::BadF);
        }
        let task = unsafe { sched::current().as_mut() }.ok_or(KernelError::Srch)?;
        let global_fd = task.fd_tbl[old_fd].ok_or(KernelError::BadF)?;
        if old_fd == new_fd {
            // Closing `new_fd` first would free the entry being duplicated.
            return Ok(new_fd as u32);
        }

        if let Some(existing) = task.fd_tbl[new_fd] {
            unsafe {
                if let Some(open_file) = fs::get_open_file(existing) {
                    fs::vfs_ref_put(open_file.node);
                }
                fs::close_open_file(existing);
            }
        }

        fs::retain_open_file(global_fd);
        if let Some(open_file) = fs::get_open_file(global_fd) {
            unsafe { fs::vfs_ref_get(open_file.node) };
        }
        task.fd_tbl[new_fd] = Some(global_fd);
        Ok(new_fd as u32)
    }
}
