use super::{VfsNode, VfsNodeType};
use crate::{
    error::{KResult, KernelError},
    pr_warn, utils,
    vga::text_mod::{print_fmt_on, print_str_on},
};

pub const MAX_VFS_NODES: usize = 1024;
pub static mut ROOT_NODE: *mut VfsNode = core::ptr::null_mut();

// static pool
static mut VFS_NODE_POOL: [VfsNode; MAX_VFS_NODES] = unsafe { core::mem::zeroed() };
static mut VFS_NODE_COUNT: usize = 0;

impl VfsNode {
    pub unsafe fn read(&self, buffer: &mut [u8], offset: u32) -> KResult<usize> {
        if self.node_type == VfsNodeType::CharDevice {
            return crate::tty::read(self.inode as usize, buffer);
        }
        if !matches!(self.node_type, VfsNodeType::File | VfsNodeType::BlockDevice) {
            pr_warn!("VFS: Attempted to read from a non-file node\n");
            return Err(KernelError::EINVAL);
        }

        let mount = (*self.master).mount;
        let fn_read = (*mount).backend.read;

        fn_read(mount, self.inode, buffer, offset)
    }

    pub unsafe fn write(&mut self, buffer: &[u8], offset: u32) -> KResult<usize> {
        if self.node_type == VfsNodeType::CharDevice {
            return crate::tty::write(self.inode as usize, buffer);
        }
        if !matches!(self.node_type, VfsNodeType::File | VfsNodeType::BlockDevice) {
            pr_warn!("VFS: Attempted to write to a non-file node\n");
            return Err(KernelError::EINVAL);
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
            pr_warn!("VFS: Attempted to truncate a non-file node\n");
            return Err(KernelError::EINVAL);
        }

        let mount = (*self.master).mount;
        let fn_truncate = (*mount).backend.truncate;

        fn_truncate(mount, self.inode)?;
        self.size = 0;
        Ok(())
    }

    pub unsafe fn lazy_load_directory(&self) -> KResult<()> {
        if self.node_type != VfsNodeType::Directory {
            pr_warn!("VFS: Attempted to lazy load a non-directory node\n");
            return Err(KernelError::EINVAL);
        }

        if !self.children.is_null() {
            return Ok(()); // Already loaded
        }

        let mount = (*self.master).mount;
        let fn_load_dir = (*mount).backend.lazy_load_directory;
        let inode = self.inode;

        fn_load_dir(mount, inode)
    }
}

pub unsafe fn alloc_vfs_node() -> KResult<*mut VfsNode> {
    if VFS_NODE_COUNT >= MAX_VFS_NODES {
        return Err(KernelError::ENFILE);
    }
    let node_ptr = &mut VFS_NODE_POOL[VFS_NODE_COUNT] as *mut VfsNode;
    VFS_NODE_COUNT += 1;
    Ok(node_ptr)
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
    let node = alloc_vfs_node()?;

    core::ptr::write_bytes((*node).name.as_mut_ptr(), 0, (*node).name.len());
    let name_bytes = name.as_bytes();
    if name_bytes.len() >= (*node).name.len() {
        return Err(KernelError::ENAMETOOLONG);
    }
    core::ptr::copy_nonoverlapping(
        name_bytes.as_ptr(),
        (*node).name.as_mut_ptr(),
        name_bytes.len(),
    );

    (*node).size = 0;
    (*node).node_type = node_type;
    (*node).inode = 0;
    (*node).links = 1;
    (*node).master = (*parent).master;
    (*node).father = parent;
    (*node).children = core::ptr::null_mut();
    (*node).next_of_kin = core::ptr::null_mut();
    (*node).rights = rights;

    append_child(parent, node);
    Ok(node)
}

pub unsafe fn mount_node(target: *mut VfsNode, mounted_root: *mut VfsNode) -> KResult<()> {
    if target.is_null() || mounted_root.is_null() {
        return Err(KernelError::EINVAL);
    }

    if (*target).node_type != VfsNodeType::Directory {
        return Err(KernelError::ENOTDIR);
    }

    (*target).master = mounted_root;
    Ok(())
}

pub unsafe fn umount_node(target: *mut VfsNode) -> KResult<()> {
    if target.is_null() {
        return Err(KernelError::EINVAL);
    }

    if (*target).node_type != VfsNodeType::Directory {
        return Err(KernelError::ENOTDIR);
    }

    (*target).master = target;
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

        if !(*node).children.is_null() {
            print_vfs_tree((*node).children, depth + 1);
        }

        node = (*node).next_of_kin;
    }
}

pub unsafe fn resolve_path(path: &str, cwd: *mut VfsNode) -> KResult<*mut VfsNode> {
    if path.is_empty() {
        return Err(KernelError::EINVAL);
    }

    let mut current = if path.starts_with('/') {
        ROOT_NODE
    } else {
        cwd
    };

    if current.is_null() {
        return Err(KernelError::EINVAL);
    }

    for segment in path.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }

        if segment == ".." {
            if !(*current).father.is_null() {
                current = (*current).father;
            }
            continue;
        }

        if (*current).node_type == VfsNodeType::Directory {
            (*current).lazy_load_directory()?;
        }

        let mut child = (*current).children;
        let mut found = false;

        while !child.is_null() {
            let name_len = (*child).name.iter().position(|&c| c == 0).unwrap_or(256);
            let child_name = core::str::from_utf8(&(*child).name[..name_len]).unwrap_or("");

            if child_name == segment {
                current = child;
                found = true;
                break;
            }
            child = (*child).next_of_kin;
        }

        if !found {
            return Err(KernelError::ENOENT);
        }
    }

    Ok(current)
}
