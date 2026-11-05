use crate::{
    error::{KResult, KernelError},
    locks::Spinlock,
    pr_warn, utils,
    vga::text_mod::{print_fmt_on, print_str_on},
};

use super::{VfsNode, VfsNodeType};

pub const MAX_VFS_NODES: usize = 1024;
pub static mut ROOT_NODE: *mut VfsNode = core::ptr::null_mut();

// Node invariants
// ---------------
// Every `*mut VfsNode` handed out by this module points into
// `VFS_NODE_POOL`, so it is always backed by memory. A node is live from
// `alloc_vfs_node` until `free_vfs_node` (through `fs::grace` for anything a
// path walk could reach; directly on failed creation). `master` points at the root node
// of the filesystem the node belongs to, whose `mount` is null for in-memory
// trees or a `MOUNT_TABLE` entry, which is never removed. Tree links
// (`father`, `children`, `next_of_kin`) are changed without a lock.

static VFS_NODE_LOCK: Spinlock<()> = Spinlock::new(());
// static pool
static mut VFS_NODE_POOL: [VfsNode; MAX_VFS_NODES] = unsafe { core::mem::zeroed() };
static mut VFS_NODE_COUNT: usize = 0;
// Singly linked through `next_of_kin`; populated by free_vfs_node().
static mut VFS_FREE_LIST: *mut VfsNode = core::ptr::null_mut();

impl VfsNode {
    pub unsafe fn read(&self, buffer: &mut [u8], offset: u32) -> KResult<usize> {
        if self.node_type == VfsNodeType::CharDevice {
            return crate::fs::dev::read_char_device(self.inode, buffer);
        }
        if self.node_type == VfsNodeType::BlockDevice {
            return crate::drivers::read_from_device(self.inode, buffer, offset);
        }
        if self.node_type == VfsNodeType::Fifo {
            let pipe_id = (self.inode >> 1) as usize;
            return crate::pipe::pipe_read(pipe_id, buffer);
        }
        if !matches!(self.node_type, VfsNodeType::File) {
            pr_warn!("VFS: Attempted to read from a non-file node");
            return Err(KernelError::Inval);
        }

        let mount = (*self.master).mount;
        let fn_read = (*mount).backend.read;

        fn_read(mount, self.inode, buffer, offset)
    }

    pub unsafe fn write(&mut self, buffer: &[u8], offset: u32) -> KResult<usize> {
        if self.node_type == VfsNodeType::CharDevice {
            return crate::fs::dev::write_char_device(self.inode, buffer);
        }
        if self.node_type == VfsNodeType::BlockDevice {
            return crate::drivers::write_to_device(self.inode, buffer, offset);
        }
        if self.node_type == VfsNodeType::Fifo {
            let pipe_id = (self.inode >> 1) as usize;
            return crate::pipe::pipe_write(pipe_id, buffer);
        }
        if !matches!(self.node_type, VfsNodeType::File) {
            pr_warn!("VFS: Attempted to write to a non-file node");
            return Err(KernelError::Inval);
        }

        let mount = (*self.master).mount;
        let fn_write = (*mount).backend.write;

        let bytes_written = fn_write(mount, self.inode, buffer, offset)?;

        // Update the VFS node if the file was appended to
        if self.node_type == VfsNodeType::File && offset + (bytes_written as u32) > self.size {
            self.size = offset + (bytes_written as u32);
        }

        Ok(bytes_written)
    }

    pub unsafe fn truncate(&mut self) -> KResult<()> {
        if !matches!(self.node_type, VfsNodeType::File) {
            pr_warn!("VFS: Attempted to truncate a non-file node");
            return Err(KernelError::Inval);
        }

        let mount = (*self.master).mount;
        let fn_truncate = (*mount).backend.truncate;

        fn_truncate(mount, self.inode)?;
        self.size = 0;
        Ok(())
    }

    /// Reads a symbolic link's target. Only pseudo-filesystem links are
    /// supported; ext2 link targets are not readable yet.
    pub unsafe fn read_link(&self, buffer: &mut [u8]) -> KResult<usize> {
        if self.node_type != VfsNodeType::Symlink || self.master.is_null() {
            return Err(KernelError::Inval);
        }
        let mount = (*self.master).mount;
        if mount.is_null() || !matches!((*mount).private_data, super::MountPrivate::Procfs) {
            return Err(KernelError::Inval);
        }
        ((*mount).backend.read)(mount, self.inode, buffer, 0)
    }

    pub unsafe fn lazy_load_directory(&self) -> KResult<()> {
        if self.node_type != VfsNodeType::Directory {
            pr_warn!("VFS: Attempted to lazy load a non-directory node");
            return Err(KernelError::Inval);
        }

        if self.loaded || !self.children.is_null() || self.inode == 0 {
            return Ok(()); // Already loaded or virtual in-memory directory
        }

        let mount = (*self.master).mount;
        if mount.is_null() {
            return Ok(());
        }
        let fn_load_dir = (*mount).backend.lazy_load_directory;
        let inode = self.inode;

        let node = self as *const VfsNode as *mut VfsNode;
        fn_load_dir(mount, node, inode)?;
        (*node).loaded = true;
        Ok(())
    }
}

pub unsafe fn alloc_vfs_node() -> KResult<*mut VfsNode> {
    let _guard = VFS_NODE_LOCK.lock();
    if !VFS_FREE_LIST.is_null() {
        let node_ptr = VFS_FREE_LIST;
        VFS_FREE_LIST = (*node_ptr).next_of_kin;
        (*node_ptr).next_of_kin = core::ptr::null_mut();
        return Ok(node_ptr);
    }
    if VFS_NODE_COUNT >= MAX_VFS_NODES {
        return Err(KernelError::NFile);
    }
    let node_ptr = &mut VFS_NODE_POOL[VFS_NODE_COUNT] as *mut VfsNode;
    (*node_ptr).ref_count = 0;
    VFS_NODE_COUNT += 1;
    Ok(node_ptr)
}

/// Returns a detached node to the pool. The caller must guarantee nothing
/// (tree links, open files, cwd) still points at it.
pub unsafe fn free_vfs_node(node: *mut VfsNode) {
    if node.is_null() {
        return;
    }
    let _guard = VFS_NODE_LOCK.lock();
    core::ptr::write_bytes(node, 0, 1);
    (*node).next_of_kin = VFS_FREE_LIST;
    VFS_FREE_LIST = node;
}

pub unsafe fn append_child(parent: *mut VfsNode, child: *mut VfsNode) {
    (*child).father = parent;

    if (*parent).children.is_null() {
        (*parent).children = child;
        return;
    }

    let mut sibling = (*parent).children;
    while !(*sibling).next_of_kin.is_null() {
        sibling = (*sibling).next_of_kin;
    }

    (*sibling).next_of_kin = child;
}

pub unsafe fn create_child_node(
    parent: *mut VfsNode,
    name: &str,
    node_type: VfsNodeType,
    rights: u16,
) -> KResult<*mut VfsNode> {
    let name_bytes = name.as_bytes();
    if name_bytes.len() >= (*parent).name.len() {
        return Err(KernelError::NameTooLong);
    }

    let node = alloc_vfs_node()?;

    core::ptr::write_bytes((*node).name.as_mut_ptr(), 0, (*node).name.len());
    core::ptr::copy_nonoverlapping(
        name_bytes.as_ptr(),
        (*node).name.as_mut_ptr(),
        name_bytes.len(),
    );

    (*node).size = 0;
    (*node).node_type = node_type;
    let credentials = crate::sched::current_cred()
        .as_ref()
        .copied()
        .unwrap_or_else(crate::sched::Credentials::root);
    let mount = (*parent).master;
    let inode = if matches!(node_type, VfsNodeType::File | VfsNodeType::Directory)
        && !mount.is_null()
        && !(*mount).mount.is_null()
    {
        let created = ((*(*mount).mount).backend.create)(
            (*mount).mount,
            (*parent).inode,
            name,
            node_type,
            rights,
            credentials.uid,
            credentials.gid,
        );
        match created {
            Ok(inode) => inode,
            Err(e) => {
                free_vfs_node(node);
                return Err(e);
            }
        }
    } else if matches!(
        node_type,
        VfsNodeType::CharDevice | VfsNodeType::BlockDevice | VfsNodeType::Socket
    ) {
        0
    } else {
        free_vfs_node(node);
        return Err(KernelError::OpNotSupp);
    };
    (*node).inode = inode;
    (*node).links = 1;
    (*node).master = (*parent).master;
    (*node).father = parent;
    (*node).children = core::ptr::null_mut();
    (*node).next_of_kin = core::ptr::null_mut();
    (*node).owner_uid = credentials.uid;
    (*node).owner_gid = credentials.gid;
    (*node).rights = rights;

    append_child(parent, node);
    vfs_ref_get(parent);
    Ok(node)
}

pub unsafe fn mount_node(target: *mut VfsNode, mounted_root: *mut VfsNode) -> KResult<()> {
    if target.is_null() || mounted_root.is_null() {
        return Err(KernelError::Inval);
    }

    if (*target).node_type != VfsNodeType::Directory {
        return Err(KernelError::NotDir);
    }

    (*target).master = mounted_root;
    vfs_ref_get(target);
    Ok(())
}

pub unsafe fn umount_node(target: *mut VfsNode) -> KResult<()> {
    if target.is_null() {
        return Err(KernelError::Inval);
    }

    if (*target).node_type != VfsNodeType::Directory {
        return Err(KernelError::NotDir);
    }

    (*target).master = target;
    vfs_ref_put(target);
    Ok(())
}

pub unsafe fn print_vfs_tree(mut node: *mut VfsNode, depth: usize) {
    while !node.is_null() {
        for _ in 0..depth {
            print_str_on(1, " |  ");
        }

        let name_str = utils::c_str_to_rust((*node).name.as_ptr());

        print_fmt_on(
            1,
            &format_args!(
                " |-- {} (Inode: {}, Type: {:?}, Size: {})\n",
                name_str,
                (*node).inode,
                (*node).node_type,
                (*node).size
            ),
        );

        if (*node).node_type == VfsNodeType::Directory
            && (*node).children.is_null()
            && (*node).inode != 0
        {
            let _ = (*node).lazy_load_directory();
        }

        if !(*node).children.is_null() {
            print_vfs_tree((*node).children, depth + 1);
        }

        node = (*node).next_of_kin;
    }
}

const MAX_SYMLINK_DEPTH: usize = 8;
const MAX_LINK_TARGET: usize = 256;

/// The pin here only covers the walk. Callers that use a file node afterwards
/// need their own `fs::pin_nodes()` around both.
pub unsafe fn resolve_path(path: &str, cwd: *mut VfsNode) -> KResult<*mut VfsNode> {
    let _pin = super::pin_nodes();
    resolve_path_at(path, cwd, true, 0)
}

pub unsafe fn resolve_path_nofollow(path: &str, cwd: *mut VfsNode) -> KResult<*mut VfsNode> {
    let _pin = super::pin_nodes();
    resolve_path_at(path, cwd, false, 0)
}

unsafe fn resolve_path_at(
    path: &str,
    cwd: *mut VfsNode,
    follow_last: bool,
    depth: usize,
) -> KResult<*mut VfsNode> {
    if path.is_empty() {
        return Err(KernelError::Inval);
    }

    let mut current = if path.starts_with('/') {
        ROOT_NODE
    } else {
        cwd
    };

    if current.is_null() {
        return Err(KernelError::Inval);
    }

    let credentials = crate::sched::current_cred()
        .as_ref()
        .copied()
        .unwrap_or_else(crate::sched::Credentials::root);
    let mut segments = path
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .peekable();
    while let Some(segment) = segments.next() {
        if segment == ".." {
            if !(*current).father.is_null() {
                current = (*current).father;
            }
            continue;
        }

        if (*current).node_type == VfsNodeType::Directory {
            (*current).lazy_load_directory()?;
            let object = crate::security::SecurityObject::File {
                owner_uid: (*current).owner_uid,
                owner_gid: (*current).owner_gid,
                mode: (*current).rights,
            };
            if crate::security::check(&credentials, &object, crate::security::Operation::Traverse)
                == crate::security::Decision::Deny
            {
                return Err(KernelError::Acces);
            }
        }

        let mut child = (*current).children;
        let mut found = false;

        while !child.is_null() {
            let name_len = (*child).name.iter().position(|&c| c == 0).unwrap_or(256);
            let child_name = core::str::from_utf8(&(&(*child).name)[..name_len]).unwrap_or("");

            if child_name == segment {
                found = true;
                break;
            }
            child = (*child).next_of_kin;
        }

        if !found {
            return Err(KernelError::NoEnt);
        }

        let is_last = segments.peek().is_none();
        if (*child).node_type == VfsNodeType::Symlink && (follow_last || !is_last) {
            let mut target = [0u8; MAX_LINK_TARGET];
            // Links whose target can't be read yet (ext2) resolve to themselves.
            if let Ok(len) = (*child).read_link(&mut target) {
                if depth >= MAX_SYMLINK_DEPTH {
                    return Err(KernelError::Loop);
                }
                let target =
                    core::str::from_utf8(&target[..len]).map_err(|_| KernelError::Inval)?;
                // Relative targets are relative to the directory holding the link.
                current = resolve_path_at(target, current, true, depth + 1)?;
                continue;
            }
        }
        current = child;
    }

    Ok(current)
}

pub unsafe fn node_path(node: *mut VfsNode, buf: &mut [u8]) -> KResult<usize> {
    if node.is_null() || node == ROOT_NODE {
        *buf.first_mut().ok_or(KernelError::Range)? = b'/';
        return Ok(1);
    }

    let mut start = buf.len();
    let mut current = node;
    while !current.is_null() && current != ROOT_NODE {
        let name_len = (*current).name.iter().position(|&c| c == 0).unwrap_or(256);
        if start < name_len + 1 {
            return Err(KernelError::NameTooLong);
        }
        start -= name_len;
        buf[start..start + name_len].copy_from_slice(&(&(*current).name)[..name_len]);
        start -= 1;
        buf[start] = b'/';
        current = (*current).father;
    }
    let len = buf.len() - start;
    buf.copy_within(start.., 0);
    Ok(len)
}

pub(crate) unsafe fn vfs_ref_get(node: *mut VfsNode) {
    if !node.is_null() {
        (*node).ref_count += 1;
    }
}

pub(crate) unsafe fn vfs_ref_put(node: *mut VfsNode) {
    if !node.is_null() && (*node).ref_count > 0 {
        (*node).ref_count -= 1;
    }
}
