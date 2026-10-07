use core::fmt::Write;

use crate::{
    error::{KResult, KernelError},
    fs::{self, VfsNodeType},
    pr_debug,
};

pub mod ops;
pub mod process;
pub mod system;

pub use ops::PROCFS_BACKEND;

pub const PROCFS_ROOT_INODE: u32 = 1;
pub const PROCFS_VERSION_INODE: u32 = 2;
pub const PROCFS_UPTIME_INODE: u32 = 3;
pub const PROCFS_MEMINFO_INODE: u32 = 4;
pub const PROCFS_CPUINFO_INODE: u32 = 5;
pub const PROCFS_STAT_INODE: u32 = 6;
pub const PROCFS_PARTITIONS_INODE: u32 = 7;
pub const PROCFS_MOUNTS_INODE: u32 = 8;
pub const PROCFS_INTERRUPTS_INODE: u32 = 9;
pub const PROCFS_CMDLINE_INODE: u32 = 10;
pub const PROCFS_DEVICES_INODE: u32 = 11;
pub const PROCFS_LOADAVG_INODE: u32 = 12;
pub const PROCFS_KMSG_INODE: u32 = 13;

pub const PROC_PID_BASE: u32 = 100;
pub const PROC_PID_STRIDE: u32 = 16;
#[allow(dead_code)]
pub const PROC_FILE_OFFSET_DIR: u32 = 0;
pub const PROC_FILE_OFFSET_STATUS: u32 = 1;
pub const PROC_FILE_OFFSET_CMDLINE: u32 = 2;
pub const PROC_FILE_OFFSET_STAT: u32 = 3;
pub const PROC_FILE_OFFSET_MAPS: u32 = 4;
pub const PROC_FILE_OFFSET_CWD: u32 = 5;

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

pub unsafe fn mount_procfs() -> KResult<()> {
    pr_debug!("Mounting procfs pseudo-filesystem at /proc...\n");

    let root = fs::ROOT_NODE;
    if root.is_null() {
        return Err(KernelError::EINVAL);
    }

    let target = fs::resolve_path("/proc", root)?;
    if (*target).node_type != VfsNodeType::Directory {
        return Err(KernelError::ENOTDIR);
    }

    let mount_ptr = fs::register_mount(|_idx| fs::Mount {
        device: 0,
        backend: &PROCFS_BACKEND,
        private_data: fs::MountPrivate::Procfs,
    })?;

    let proc_root = fs::alloc_vfs_node()?;
    core::ptr::write_bytes((*proc_root).name.as_mut_ptr(), 0, 256);
    core::ptr::copy_nonoverlapping(b"proc".as_ptr(), (*proc_root).name.as_mut_ptr(), 4);
    (*proc_root).inode = PROCFS_ROOT_INODE;
    (*proc_root).node_type = VfsNodeType::Directory;
    (*proc_root).rights = 0o555;
    (*proc_root).owner_uid = 0;
    (*proc_root).owner_gid = 0;
    (*proc_root).size = 0;
    (*proc_root).links = 2;
    (*proc_root).master = proc_root;
    (*proc_root).father = (*target).father;
    (*proc_root).children = core::ptr::null_mut();
    (*proc_root).next_of_kin = core::ptr::null_mut();
    (*proc_root).mount = mount_ptr;

    (*target).master = proc_root;
    (*target).mount = mount_ptr;

    // Pre-populate static entries and initial PIDs onto target
    ops::populate_proc_root(target, proc_root)?;

    pr_debug!("procfs mounted successfully at /proc\n");
    Ok(())
}
