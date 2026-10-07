use core::fmt::Write;

use crate::{
    error::{KResult, KernelError},
    fs::{self, VfsNodeType},
    pr_info,
};

pub mod ops;

pub use ops::SYSFS_BACKEND;

pub const SYSFS_ROOT_INODE: u32 = 1;
pub const SYSFS_KERNEL_DIR_INODE: u32 = 2;
pub const SYSFS_MEM_DIR_INODE: u32 = 3;
pub const SYSFS_SYS_DIR_INODE: u32 = 4;
pub const SYSFS_MEM_STAT_INODE: u32 = 5;
pub const SYSFS_MEM_DEBUG_INODE: u32 = 6;
pub const SYSFS_INITCALLS_INODE: u32 = 7;
pub const SYSFS_INFO_INODE: u32 = 8;

pub(crate) struct BufferWriter<'a> {
    buf: &'a mut [u8],
    offset: usize,
}

impl<'a> BufferWriter<'a> {
    pub(crate) fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, offset: 0 }
    }

    pub(crate) fn written(&self) -> usize {
        self.offset
    }
}

impl<'a> Write for BufferWriter<'a> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let remaining = self.buf.len().saturating_sub(self.offset);
        let to_copy = remaining.min(bytes.len());
        if to_copy > 0 {
            self.buf[self.offset..self.offset + to_copy].copy_from_slice(&bytes[..to_copy]);
            self.offset += to_copy;
        }
        Ok(())
    }
}

pub unsafe fn mount_sysfs() -> KResult<()> {
    pr_info!("Mounting sysfs pseudo-filesystem at /sys...\n");

    let root = fs::ROOT_NODE;
    if root.is_null() {
        return Err(KernelError::EINVAL);
    }

    let target = fs::resolve_path("/sys", root)?;
    if (*target).node_type != VfsNodeType::Directory {
        return Err(KernelError::ENOTDIR);
    }

    let mount_ptr = fs::register_mount(|_idx| fs::Mount {
        device: 0,
        backend: &SYSFS_BACKEND,
        private_data: fs::MountPrivate::Sysfs,
    })?;

    fs::mount_node(target, target)?;
    (*target).mount = mount_ptr;
    (*target).inode = SYSFS_ROOT_INODE;

    ops::populate_sysfs_root(target, target)?;

    pr_info!("sysfs mounted successfully at /sys\n");
    Ok(())
}
