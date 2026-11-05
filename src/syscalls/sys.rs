use super::args::{UserAddr, UserSlice, caller, syscall_define};
use crate::{
    error::KernelError,
    fs::{self, dev},
    sched, security,
};

syscall_define! {
    pub(super) fn syscall_getuid() {
        Ok(caller()?.credentials.uid)
    }
}

syscall_define! {
    pub(super) fn syscall_login(user: UserAddr, user_len: usize, pass: UserAddr, pass_len: usize) {
        let username = UserSlice::new(user, user_len)?;
        let password = UserSlice::new(pass, pass_len)?;

        if unsafe { security::login_current(username.as_bytes(), password.as_bytes()) } {
            Ok(0)
        } else {
            Err(KernelError::Acces)
        }
    }
}

syscall_define! {
    /// copies the caller's user name (not NULL terminated, truncated to `size`)
    pub(super) fn syscall_getusername(buf: UserAddr, size: usize) {
        let mut out = UserSlice::new(buf, size)?;
        let uid = caller()?.credentials.uid;

        let mut name = [0u8; 64];
        let name: &[u8] = match security::username_for_uid(uid, &mut name) {
            Some(len) => &name[..len],
            None if uid == 0 => b"root",
            None => return Err(KernelError::NoEnt),
        };
        let len = name.len().min(size);
        out.as_bytes_mut()[..len].copy_from_slice(&name[..len]);
        Ok(len as u32)
    }
}

syscall_define! {
    /// copies the name of the tty `fd` refers to
    /// (`ttyN`, not NUL terminated, truncated to `size`).
    pub(super) fn syscall_getttyname(fd: usize, buf: UserAddr, size: usize) {
        let mut out = UserSlice::new(buf, size)?;
        if fd >= sched::MAX_FDS_PER_PROCESS {
            return Err(KernelError::BadF);
        }
        let global_fd = unsafe { sched::current().as_ref() }.ok_or(KernelError::Srch)?.fd_tbl[fd]
            .ok_or(KernelError::BadF)?;
        let open_file = fs::get_open_file(global_fd).ok_or(KernelError::BadF)?;
        let node = unsafe { &*open_file.node };
        if node.node_type != fs::VfsNodeType::CharDevice {
            return Err(KernelError::NotTTY);
        }
        let index = match node.inode {
            inode @ 0..=5 => inode as usize,
            dev::CHAR_DEV_TTY0 | dev::CHAR_DEV_CONSOLE => crate::vga::text_mod::active_screen_index(),
            dev::CHAR_DEV_TTY => dev::get_controlling_tty_index(),
            _ => return Err(KernelError::NotTTY),
        };

        let (name, name_len) = crate::tty::tty_node_name(index);
        let len = name_len.min(size);
        out.as_bytes_mut()[..len].copy_from_slice(&name[..len]);
        Ok(len as u32)
    }
}
