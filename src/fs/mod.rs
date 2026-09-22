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

unsafe impl Send for OpenFile {}

pub const MAX_OPEN_FILES: usize = 256;
pub static OPEN_FILE_TABLE: Spinlock<[Option<OpenFile>; MAX_OPEN_FILES]> = {
    const EMPTY: Option<OpenFile> = None;
    Spinlock::new([EMPTY; MAX_OPEN_FILES])
};

pub fn alloc_open_file(node: *mut VfsNode, ref_count: usize) -> KResult<usize> {
    let mut table = OPEN_FILE_TABLE.lock();
    for (i, slot) in table.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = Some(OpenFile { node, offset: 0, ref_count });
            return Ok(i);
        }
    }
    Err(KernelError::ENFILE)
}

pub fn get_open_file(global_fd: usize) -> Option<OpenFile> {
    if global_fd >= MAX_OPEN_FILES {
        return None;
    }
    let table = OPEN_FILE_TABLE.lock();
    table[global_fd]
}

pub fn retain_open_file(global_fd: usize) {
    if global_fd >= MAX_OPEN_FILES {
        return;
    }
    let mut table = OPEN_FILE_TABLE.lock();
    if let Some(Some(ref mut open_file)) = table.get_mut(global_fd) {
        open_file.ref_count += 1;
    }
}

pub fn update_open_file_offset(global_fd: usize, delta: u32) {
    if global_fd >= MAX_OPEN_FILES {
        return;
    }
    let mut table = OPEN_FILE_TABLE.lock();
    if let Some(Some(ref mut open_file)) = table.get_mut(global_fd) {
        open_file.offset = open_file.offset.saturating_add(delta);
    }
}

pub unsafe fn close_open_file(global_fd: usize) {
    if global_fd >= MAX_OPEN_FILES {
        return;
    }
    let mut table = OPEN_FILE_TABLE.lock();
    let should_close = if let Some(ref mut open_file) = table[global_fd] {
        if open_file.ref_count > 0 {
            open_file.ref_count -= 1;
        }
        if open_file.ref_count == 0 {
            let node = open_file.node;
            table[global_fd] = None;
            Some(node)
        } else {
            None
        }
    } else {
        None
    };
    drop(table);

    if let Some(node) = should_close {
        if !node.is_null() && (*node).node_type == VfsNodeType::Socket {
            let sock_idx = (*node).inode as usize;
            crate::ipc::close_socket(sock_idx);
        }
    }
}

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

    pub owner_uid: u32,
    pub owner_gid: u32,
    pub rights: u16,

    pub mount: *const Mount, // if this node is a mount point
}

#[derive(Clone, Copy)]
pub struct Mount {
    #[allow(dead_code)]
    pub device: BlockDeviceId,
    pub backend: &'static FsBackend,
    pub private_data: MountPrivate,
}

#[derive(Clone, Copy)]
pub enum MountPrivate {
    Ext2(Ext2Mount),
    #[allow(dead_code)]
    Raw,
}

pub struct FsBackend {
    pub create: unsafe fn(
        mount: *const Mount,
        parent_inode: u32,
        name: &str,
        node_type: VfsNodeType,
        mode: u16,
        uid: u32,
        gid: u32,
    ) -> KResult<u32>,
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
    pub lazy_load_directory:
        unsafe fn(mount: *const Mount, dir_node: *mut VfsNode, dir_node_num: u32) -> KResult<()>,
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

#[allow(dead_code)]
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
