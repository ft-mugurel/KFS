use crate::{
    error::KernelError,
    fs::{self, VfsNodeType},
    sched::{self, ContextFrame},
    utils,
};

pub unsafe fn syscall_chdir(regs: *mut ContextFrame) {
    let path_ptr = (*regs).arg1() as *const u8;
    if path_ptr.is_null() {
        (*regs).set_return_error(KernelError::EFAULT);
        return;
    }
    let path_str = utils::c_str_to_rust(path_ptr);
    let task = match sched::current().as_mut() {
        Some(t) => t,
        None => {
            (*regs).set_return_error(KernelError::ESRCH);
            return;
        }
    };

    let target = match fs::resolve_path(path_str, task.cwd) {
        Ok(node) => node,
        Err(err) => {
            (*regs).set_return_error(err);
            return;
        }
    };

    if (*target).node_type != VfsNodeType::Directory {
        (*regs).set_return_error(KernelError::ENOTDIR);
        return;
    }

    task.cwd = target;
    (*regs).set_return_value(0);
}

pub unsafe fn syscall_getcwd(regs: *mut ContextFrame) {
    let buf_ptr = (*regs).arg1() as *mut u8;
    let size = (*regs).arg2() as usize;
    if buf_ptr.is_null() || size == 0 {
        (*regs).set_return_error(KernelError::EINVAL);
        return;
    }
    let task = match sched::current().as_mut() {
        Some(t) => t,
        None => {
            (*regs).set_return_error(KernelError::ESRCH);
            return;
        }
    };

    let mut current = task.cwd;
    if current.is_null() || current == fs::ROOT_NODE {
        if size < 2 {
            (*regs).set_return_error(KernelError::ERANGE);
            return;
        }
        *buf_ptr = b'/';
        *buf_ptr.add(1) = 0;
        (*regs).set_return_value(1);
        return;
    }

    let mut segments: [*mut fs::VfsNode; 16] = [core::ptr::null_mut(); 16];
    let mut count = 0;

    while !current.is_null() && current != fs::ROOT_NODE && count < 16 {
        segments[count] = current;
        count += 1;
        current = (*current).father;
    }

    let mut offset = 0;
    for i in (0..count).rev() {
        let node = segments[i];
        let name_len = (*node)
            .name
            .iter()
            .position(|&c| c == 0)
            .unwrap_or((*node).name.len());
        if offset + 1 + name_len >= size {
            (*regs).set_return_error(KernelError::ERANGE);
            return;
        }
        *buf_ptr.add(offset) = b'/';
        offset += 1;
        core::ptr::copy_nonoverlapping((*node).name.as_ptr(), buf_ptr.add(offset), name_len);
        offset += name_len;
    }

    if offset >= size {
        (*regs).set_return_error(KernelError::ERANGE);
        return;
    }
    *buf_ptr.add(offset) = 0;
    (*regs).set_return_value(offset as u32);
}
