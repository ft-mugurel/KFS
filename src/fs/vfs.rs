use super::{VfsNode, VfsNodeType};
use crate::{
    drivers,
    error::{KResult, KernelError},
    fs::ext2::EXT2_BLOCK_SIZE,
    pr_err, pr_info, utils,
};

pub const MAX_VFS_NODES: usize = 1024;
pub static mut ROOT_NODE: *mut VfsNode = core::ptr::null_mut();

// static pool
static mut VFS_NODE_POOL: [VfsNode; MAX_VFS_NODES] = unsafe { core::mem::zeroed() };
static mut VFS_NODE_COUNT: usize = 0;

pub unsafe fn alloc_vfs_node() -> KResult<*mut VfsNode> {
    if VFS_NODE_COUNT >= MAX_VFS_NODES {
        return Err(KernelError::ENFILE);
    }
    let node_ptr = &mut VFS_NODE_POOL[VFS_NODE_COUNT] as *mut VfsNode;
    VFS_NODE_COUNT += 1;
    Ok(node_ptr)
}

// Helper to convert Ext2 mode/type to VfsNodeType
pub fn ext2_type_to_vfs(ext2_type: u8) -> VfsNodeType {
    match ext2_type {
        1 => VfsNodeType::File,
        2 => VfsNodeType::Directory,
        7 => VfsNodeType::Symlink,
        _ => VfsNodeType::Unknown,
    }
}

pub unsafe fn print_vfs_tree(mut node: *mut VfsNode, depth: usize) {
    while !node.is_null() {
        for _ in 0..depth {
            pr_info!("|   ");
        }

        let name_str = utils::c_str_to_rust((*node).name.as_ptr());

        pr_info!(
            "|-- {} (Inode: {}, Type: {:?}, Size: {})\n",
            name_str,
            (*node).inode,
            (*node).node_type,
            (*node).size
        );

        if !(*node).children.is_null() {
            print_vfs_tree((*node).children, depth + 1);
        }

        node = (*node).next_of_kin;
    }
}

pub unsafe fn lazy_load_directory(dir_node: *mut VfsNode) -> KResult<()> {
    if !(*dir_node).children.is_null() {
        return Ok(()); // Already loaded
    }

    let inode = super::get_ext2_inode((*dir_node).inode)?;
    // For simplicity, we only read the first direct block of the directory.
    // Large directories require reading i_block[1], i_block[2], etc.
    let block_num = inode.i_block[0];
    if block_num == 0 {
        return Ok(());
    }

    let block_size = EXT2_BLOCK_SIZE;
    let lba = block_num * (block_size / 512);

    let mut dir_buf: [u8; 4096] = [0; 4096];
    drivers::read_sectors(lba, (block_size / 512) as u8, &mut dir_buf)?;

    crate::fs::ext2::parse_directory_block(
        dir_buf.as_ptr(),
        block_size as usize,
        dir_node,
        (*dir_node).master,
    );
    Ok(())
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
            lazy_load_directory(current)?;
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

pub unsafe fn read_file(node: *mut VfsNode, buffer: &mut [u8], offset: u32) -> KResult<usize> {
    if (*node).node_type != VfsNodeType::File {
        return Err(KernelError::EINVAL);
    }

    let inode = super::get_ext2_inode((*node).inode)?;

    let file_size = inode.i_size;
    if offset >= file_size {
        return Ok(0); // EOF
    }

    let bytes_to_read = core::cmp::min(buffer.len() as u32, file_size - offset);

    let block_size = super::EXT2_BLOCK_SIZE;
    let logical_block_idx = (offset / block_size) as usize;
    let offset_in_block = (offset % block_size) as usize;

    if logical_block_idx > 11 {
        pr_err!("File too large, indirect blocks not implemented.\n");
        return Err(KernelError::ENOSYS);
    }

    let physical_block = inode.i_block[logical_block_idx];
    if physical_block == 0 {
        return Ok(0); // Sparse file, treat as EOF
    }

    let mut block_buf: [u8; 1024] = [0; 1024];
    if super::read_ext2_block(physical_block, &mut block_buf).is_err() {
        return Err(KernelError::EIO);
    }

    core::ptr::copy_nonoverlapping(
        block_buf.as_ptr().add(offset_in_block),
        buffer.as_mut_ptr(),
        bytes_to_read as usize,
    );

    Ok(bytes_to_read as usize)
}
