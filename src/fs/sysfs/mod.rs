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
pub const SYSFS_DEBUG_DIR_INODE: u32 = 3;
pub const SYSFS_DEBUG_MEMSTAT_INODE: u32 = 4;
pub const SYSFS_DEBUG_MEMDEBUG_INODE: u32 = 5;
pub const SYSFS_DEBUG_INITCALLS_INODE: u32 = 6;
pub const SYSFS_DEBUG_SYSINFO_INODE: u32 = 7;
pub const SYSFS_MODULE_DIR_INODE: u32 = 8;
pub const SYSFS_DEVICES_DIR_INODE: u32 = 9;
pub const SYSFS_SYSTEM_DIR_INODE: u32 = 10;
pub const SYSFS_CPU_DIR_INODE: u32 = 11;
pub const SYSFS_CPU_ONLINE_INODE: u32 = 12;
pub const SYSFS_CPU_POSSIBLE_INODE: u32 = 13;
pub const SYSFS_CPU_PRESENT_INODE: u32 = 14;
pub const SYSFS_BLOCK_DIR_INODE: u32 = 15;

// /sys/module/<name> is SYSFS_MODULE_BASE + index * SYSFS_MODULE_STRIDE, plus:
pub const SYSFS_MODULE_BASE: u32 = 100;
pub const SYSFS_MODULE_STRIDE: u32 = 4;
pub const SYSFS_MODULE_STATE: u32 = 1;

// /sys/block/<dev> is SYSFS_BLOCK_BASE + device_id * SYSFS_BLOCK_STRIDE, plus:
pub const SYSFS_BLOCK_BASE: u32 = 200;
pub const SYSFS_BLOCK_STRIDE: u32 = 4;
pub const SYSFS_BLOCK_SIZE: u32 = 1;
pub const SYSFS_BLOCK_START: u32 = 2;

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
    pr_info!("Mounting sysfs pseudo-filesystem at /sys...");

    let root = fs::ROOT_NODE;
    if root.is_null() {
        return Err(KernelError::Inval);
    }

    let target = fs::resolve_path("/sys", root)?;
    if (*target).node_type != VfsNodeType::Directory {
        return Err(KernelError::NotDir);
    }

    let mount_ptr = fs::register_mount(|_idx| fs::Mount {
        device: 0,
        mount_point: "/sys",
        backend: &SYSFS_BACKEND,
        private_data: fs::MountPrivate::Sysfs,
    })?;

    fs::mount_node(target, target)?;
    (*target).mount = mount_ptr;
    (*target).inode = SYSFS_ROOT_INODE;

    ops::populate_sysfs_root(target, target)?;

    pr_info!("sysfs mounted successfully at /sys");
    Ok(())
}
