use crate::{
    drivers::BlockDeviceId,
    error::{KResult, KernelError},
    fs::ext2::Ext2Mount,
    locks::Spinlock,
};

#[derive(Debug, Clone, Copy)]
pub struct OpenFile {
    pub node: *mut VfsNode,
    pub offset: u32,
    pub ref_count: usize,
}

pub const MAX_OPEN_FILES: usize = 256;
pub static mut OPEN_FILE_TABLE: [Option<OpenFile>; MAX_OPEN_FILES] = {
    const EMPTY: Option<OpenFile> = None;
    [EMPTY; MAX_OPEN_FILES]
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfsNodeType {
    Unknown = 0,
    Fifo = 1,
    CharDevice = 2,
    Directory = 3,
    BlockDevice = 4,
    File = 5,
    Socket = 6,
    Symlink = 7,
}

pub struct VfsNode {
    pub name: [u8; 256],
    pub size: u32,
    pub node_type: VfsNodeType,
    pub inode: u32,
    pub links: u32,
    pub master: *mut VfsNode,
    pub father: *mut VfsNode,
    pub children: *mut VfsNode,
    pub next_of_kin: *mut VfsNode,

    pub rights: u16, // don't have users/groups yet

    pub mount: *const Mount, // if this node is a mount point
}

#[derive(Clone, Copy)]
pub struct Mount {
    pub device: BlockDeviceId,
    pub backend: &'static FsBackend,
    pub private_data: MountPrivate,
}

#[derive(Clone, Copy)]
pub enum MountPrivate {
    Ext2(Ext2Mount),
    Raw,
}

pub struct FsBackend {
    pub read: unsafe fn(
        mount: *const Mount,
        inode_num: u32,
        buffer: &mut [u8],
        offset: u32,
    ) -> KResult<usize>,
    pub write: unsafe fn(
        mount: *const Mount,
        inode_num: u32,
        buffer: &[u8],
        offset: u32,
    ) -> KResult<usize>,
    pub truncate: unsafe fn(mount: *const Mount, inode_num: u32) -> KResult<()>,
    pub lazy_load_directory: unsafe fn(mount: *const Mount, dir_node_num: u32) -> KResult<()>,
}

const MOUNT_TABLE_SIZE: usize = 16;
static MOUNT_TABLE: Spinlock<[Option<Mount>; MOUNT_TABLE_SIZE]> =
    Spinlock::new([None; MOUNT_TABLE_SIZE]);

pub(self) fn register_mount<F>(create_mount: F) -> KResult<*const Mount>
where
    F: FnOnce(usize) -> Mount,
{
    let mut table = MOUNT_TABLE.lock();
    for (index, entry) in table.iter_mut().enumerate() {
        if entry.is_none() {
            *entry = Some(create_mount(index));
            return Ok(entry.as_ref().unwrap() as *const Mount);
        }
    }
    Err(KernelError::ENOSPC)
}

pub(self) fn get_mount(index: usize) -> KResult<Mount> {
    let table = MOUNT_TABLE.lock();
    if index < MOUNT_TABLE_SIZE {
        if let Some(mount) = table[index] {
            return Ok(mount);
        }
    }
    Err(KernelError::EINVAL)
}

// mod buffer_cache;
mod vfs;

pub mod ext2;

pub use vfs::{
    alloc_vfs_node, create_child_node, mount_node, print_vfs_tree, resolve_path, umount_node,
    ROOT_NODE,
};
