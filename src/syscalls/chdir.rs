use super::args::{PATH_MAX, UserAddr, UserPtr, UserSlice, caller, syscall_define};
use crate::{
    error::KernelError,
    fs::{self, VfsNodeType},
    sched,
};

syscall_define! {
    pub fn syscall_chdir(path: UserPtr<u8>) {
        let path = path.read_cstr(PATH_MAX)?;
        let caller = caller()?;
        let _pin = fs::pin_nodes();

        let target = unsafe { fs::resolve_path(path, caller.cwd) }?;
        if unsafe { (*target).node_type } != VfsNodeType::Directory {
            return Err(KernelError::NotDir);
        }

        unsafe { sched::current().as_mut() }.ok_or(KernelError::Srch)?.cwd = target;
        Ok(0)
    }
}

syscall_define! {
    pub fn syscall_getcwd(buf: UserAddr, size: usize) {
        if size == 0 {
            return Err(KernelError::Inval);
        }
        let mut out = UserSlice::new(buf, size)?;
        let caller = caller()?;

        let mut path = [0u8; PATH_MAX];
        let len = unsafe { fs::node_path(caller.cwd, &mut path) }?;
        if len + 1 > size {
            return Err(KernelError::Range);
        }
        let out = out.as_bytes_mut();
        out[..len].copy_from_slice(&path[..len]);
        out[len] = 0;
        Ok(len as u32)
    }
}

syscall_define! {
    /// readlink(path, buf, bufsiz): copies the link target (not NUL terminated).
    pub fn syscall_readlink(path: UserPtr<u8>, buf: UserAddr, size: usize) {
        if size == 0 {
            return Err(KernelError::Inval);
        }
        let mut out = UserSlice::new(buf, size)?;
        let path = path.read_cstr(PATH_MAX)?;
        let caller = caller()?;
        let _pin = fs::pin_nodes();

        let node = unsafe { fs::resolve_path_nofollow(path, caller.cwd) }?;
        let mut target = [0u8; PATH_MAX];
        let len = unsafe { (*node).read_link(&mut target) }?.min(size).min(target.len());
        out.as_bytes_mut()[..len].copy_from_slice(&target[..len]);
        Ok(len as u32)
    }
}
