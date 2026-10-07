use super::read_write::valid_user_buffer;
use crate::{
    error::KernelError,
    fs,
    sched::{self, ContextFrame},
    security,
};

pub(super) unsafe fn syscall_getuid(regs: *mut ContextFrame) {
    (*regs).set_return_value(sched::current().as_ref().unwrap().credentials.uid);
}

pub(super) unsafe fn syscall_login(regs: *mut ContextFrame) {
    let user_ptr = (*regs).arg1() as *const u8;
    let user_len = (*regs).arg2() as usize;
    let pass_ptr = (*regs).arg3() as *const u8;
    let pass_len = (*regs).arg4() as usize;

    if !valid_user_buffer(user_ptr as u32, user_len)
        || !valid_user_buffer(pass_ptr as u32, pass_len)
    {
        (*regs).set_return_error(KernelError::EFAULT);
        return;
    }

    let username = core::slice::from_raw_parts(user_ptr, user_len);
    let password = core::slice::from_raw_parts(pass_ptr, pass_len);

    if security::login_current(username, password) {
        (*regs).set_return_value(0);
    } else {
        (*regs).set_return_error(KernelError::EACCES);
    }
}

pub(super) unsafe fn syscall_getusername(regs: *mut ContextFrame) {
    let buf_ptr = (*regs).arg1() as *mut u8;
    let buf_len = (*regs).arg2() as usize;

    if !valid_user_buffer(buf_ptr as u32, buf_len) {
        (*regs).set_return_error(KernelError::EFAULT);
        return;
    }

    let uid = sched::current().as_ref().unwrap().credentials.uid;
    let mut kbuf = [0u8; 64];
    if let Some(len) = security::username_for_uid(uid, &mut kbuf) {
        let copy_len = len.min(buf_len);
        core::ptr::copy_nonoverlapping(kbuf.as_ptr(), buf_ptr, copy_len);
        (*regs).set_return_value(copy_len as u32);
    } else if uid == 0 {
        let root = b"root";
        let copy_len = root.len().min(buf_len);
        core::ptr::copy_nonoverlapping(root.as_ptr(), buf_ptr, copy_len);
        (*regs).set_return_value(copy_len as u32);
    } else {
        (*regs).set_return_error(KernelError::ENOENT);
    }
}

pub(super) unsafe fn syscall_getttyname(regs: *mut ContextFrame) {
    let fd = (*regs).arg1() as usize;
    let buf_ptr = (*regs).arg2() as *mut u8;
    let buf_len = (*regs).arg3() as usize;

    if !valid_user_buffer(buf_ptr as u32, buf_len) {
        (*regs).set_return_error(KernelError::EFAULT);
        return;
    }

    if fd >= sched::MAX_FDS_PER_PROCESS {
        (*regs).set_return_error(KernelError::EBADF);
        return;
    }

    let task = sched::current().as_ref().unwrap();
    let global_fd = match task.fd_tbl[fd] {
        Some(gfd) => gfd,
        None => {
            (*regs).set_return_error(KernelError::EBADF);
            return;
        }
    };

    let open_file = match fs::get_open_file(global_fd) {
        Some(f) => f,
        None => {
            (*regs).set_return_error(KernelError::EBADF);
            return;
        }
    };

    let node = open_file.node;
    if (*node).node_type != fs::VfsNodeType::CharDevice {
        (*regs).set_return_error(KernelError::ENOTTY);
        return;
    }

    let tty_idx = (*node).inode + 1;
    let mut name_buf = [0u8; 16];
    let name_len = {
        name_buf[0] = b't';
        name_buf[1] = b't';
        name_buf[2] = b'y';
        if tty_idx >= 10 {
            name_buf[3] = b'0' + (tty_idx / 10) as u8;
            name_buf[4] = b'0' + (tty_idx % 10) as u8;
            5
        } else {
            name_buf[3] = b'0' + tty_idx as u8;
            4
        }
    };

    let copy_len = name_len.min(buf_len);
    core::ptr::copy_nonoverlapping(name_buf.as_ptr(), buf_ptr, copy_len);
    (*regs).set_return_value(copy_len as u32);
}
