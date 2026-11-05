// It shouldn't have a serious problem, I'll come back to this module
#![allow(unsafe_op_in_unsafe_fn)]

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
    Err(KernelError::NFile)
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
    if let Some(Some(open_file)) = table.get_mut(global_fd) {
        open_file.ref_count += 1;
    }
}

pub fn update_open_file_offset(global_fd: usize, delta: u32) {
    if global_fd >= MAX_OPEN_FILES {
        return;
    }
    let mut table = OPEN_FILE_TABLE.lock();
    if let Some(Some(open_file)) = table.get_mut(global_fd) {
        open_file.offset = open_file.offset.saturating_add(delta);
    }
}

/// # Safety
/// The caller must own one reference to `global_fd` (it is dropped here) and
/// must not hold OPEN_FILE_TABLE.
pub unsafe fn close_open_file(global_fd: usize) {
    if global_fd >= MAX_OPEN_FILES {
        return;
    }
    let mut table = OPEN_FILE_TABLE.lock();
    let Some(open_file) = table[global_fd].as_mut() else {
        return;
    };
    open_file.ref_count = open_file.ref_count.saturating_sub(1);
    if open_file.ref_count != 0 {
        return;
    }
    let node = open_file.node;
    table[global_fd] = None;
    if node.is_null() {
        return;
    }

    // Decided under the lock so that only one closer can free the node. It
    // may belong to a later closer after unlocking, so copy what's needed.
    let (node_type, inode) = ((*node).node_type, (*node).inode);
    let free_node = is_orphan(node) && !node_is_open(&*table, node) && claim_free(node);
    drop(table);

    if node_type == VfsNodeType::Socket {
        crate::ipc::close_socket(inode as usize);
    } else if node_type == VfsNodeType::Fifo {
        let pipe_id = (inode >> 1) as usize;
        if inode & 1 == 0 {
            crate::pipe::close_read_end(pipe_id);
        } else {
            crate::pipe::close_write_end(pipe_id);
        }
    }

    if free_node {
        grace::queue_free(node);
    }
}

/// Outside the tree, so open files are the only things pointing at it: pipes,
/// sockets and unlinked files. Directories are excluded because a task's `cwd`
/// can be inside one without an open file.
unsafe fn is_orphan(node: *mut VfsNode) -> bool {
    (*node).father.is_null() && (*node).node_type != VfsNodeType::Directory
}

fn node_is_open(table: &[Option<OpenFile>], node: *mut VfsNode) -> bool {
    table.iter().flatten().any(|file| file.node == node)
}

/// Marks `node` as queued for freeing, so that it is only queued once. Returns
/// whether the caller is the one to queue it. OPEN_FILE_TABLE must be held.
unsafe fn claim_free(node: *mut VfsNode) -> bool {
    !core::mem::replace(&mut (*node).free_pending, true)
}

/// Removes `child`, named `name`, from the backing filesystem and the tree.
///
/// # Safety
/// As for `detach_child`, and the caller must hold a `NodePin`.
pub unsafe fn unlink_child(parent: *mut VfsNode, name: &str, child: *mut VfsNode) -> KResult<()> {
    // Device nodes only exist in memory, whatever directory they sit in.
    let in_memory = matches!(
        (*child).node_type,
        VfsNodeType::CharDevice | VfsNodeType::BlockDevice | VfsNodeType::Socket | VfsNodeType::Fifo
    );
    let master = (*parent).master;
    let mount = if master.is_null() { core::ptr::null() } else { (*master).mount };
    if !in_memory && !mount.is_null() {
        ((*mount).backend.unlink)(mount, (*parent).inode, name)?;
        (*child).links = (*child).links.saturating_sub(1);
    }
    detach_child(parent, child)
}

/// Removes `child` from `parent`'s children, and frees it unless it is still
/// open, in which case the last close does. A second unlink of the same child
/// gets `NoEnt`.
///
/// # Safety
/// `parent` must be a live directory, and `child` must not be one (a cwd can be
/// inside it). The caller must not hold any spinlock.
pub unsafe fn detach_child(parent: *mut VfsNode, child: *mut VfsNode) -> KResult<()> {
    let table = OPEN_FILE_TABLE.lock();

    let mut link: *mut *mut VfsNode = &mut (*parent).children;
    while *link != child {
        if (*link).is_null() {
            return Err(KernelError::NoEnt);
        }
        link = &mut (**link).next_of_kin;
    }
    // `next_of_kin` stays: a pinned walker may be standing on this node.
    *link = (*child).next_of_kin;
    (*child).father = core::ptr::null_mut();
    // It had a child, so it's loaded, even if it is empty now.
    (*parent).loaded = true;
    let free_node = !node_is_open(&*table, child) && claim_free(child);
    drop(table);

    if free_node {
        grace::queue_free(child);
    }
    Ok(())
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
    pub ref_count: u32,
    pub master: *mut VfsNode,
    pub father: *mut VfsNode,
    pub children: *mut VfsNode,
    pub next_of_kin: *mut VfsNode,

    pub owner_uid: u32,
    pub owner_gid: u32,
    pub rights: u16,

    pub mount: *const Mount, // if this node is a mount point

    pub loaded: bool, // children have been read from the backend
    pub free_pending: bool, // queued in fs::grace; guarded by OPEN_FILE_TABLE
    pub next_free: *mut VfsNode, // link in fs::grace's queues
}

#[derive(Clone, Copy)]
pub struct Mount {
    pub device: BlockDeviceId,
    pub mount_point: &'static str,
    pub backend: &'static FsBackend,
    pub private_data: MountPrivate,
}

#[derive(Clone, Copy)]
pub enum MountPrivate {
    Ext2(Ext2Mount),
    Procfs,
    Sysfs,
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
    pub unlink: unsafe fn(mount: *const Mount, parent_inode: u32, name: &str) -> KResult<()>,
    /// The last VFS node for an unlinked inode went away.
    pub evict: unsafe fn(mount: *const Mount, inode_num: u32) -> KResult<()>,
}

const MOUNT_TABLE_SIZE: usize = 16;
static MOUNT_TABLE: Spinlock<[Option<Mount>; MOUNT_TABLE_SIZE]> =
    Spinlock::new([None; MOUNT_TABLE_SIZE]);

pub(crate) fn register_mount<F>(create_mount: F) -> KResult<*const Mount>
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
    Err(KernelError::NoSpc)
}

pub fn for_each_mount<F: FnMut(&Mount)>(mut f: F) {
    let table = MOUNT_TABLE.lock();
    for entry in table.iter().flatten() {
        f(entry);
    }
}

pub mod buffer_cache;
pub mod seq;
mod vfs;

pub mod dev;
pub mod ext2;
mod grace;
pub mod hierarchy;
pub mod procfs;
pub mod sysfs;

pub use vfs::{
    MAX_VFS_NODES, ROOT_NODE, alloc_vfs_node, append_child, create_child_node, free_vfs_node, mount_node,
    node_path, print_vfs_tree, resolve_path, resolve_path_nofollow, umount_node,
};
pub(crate) use grace::{defer_free, pin_nodes, release_task_pins};
pub(crate) use vfs::{vfs_ref_get, vfs_ref_put};

static mut ROOT_FILESYSTEM_READY: bool = false;
static mut PERSISTENT_HIERARCHY_READY: bool = false;
static mut RUNTIME_FILESYSTEMS_READY: bool = false;
static mut DEV_READY: bool = false;

pub fn root_filesystem_ready() -> bool {
    unsafe {
        ROOT_FILESYSTEM_READY
            && PERSISTENT_HIERARCHY_READY
            && RUNTIME_FILESYSTEMS_READY
            && DEV_READY
    }
}

pub unsafe fn mark_persistent_hierarchy_ready() {
    PERSISTENT_HIERARCHY_READY = true;
}

pub unsafe fn mark_runtime_filesystems_ready() {
    RUNTIME_FILESYSTEMS_READY = true;
}

pub unsafe fn mark_dev_ready() {
    DEV_READY = true;
}
#[unsafe(link_section = ".init.text")]
pub fn init_root_filesystem() -> KResult<()> {
    let device_id = crate::drivers::first_ext2_partition().ok_or(KernelError::NoDev)?;

    unsafe {
        ext2::mount_device(device_id)?;
        hierarchy::init_unix_hierarchy()?;
        ROOT_FILESYSTEM_READY = true;
    }

    Ok(())
}

crate::fs_initcall!(init_root_filesystem);
