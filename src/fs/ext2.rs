use super::{
    Ext2DirEntry, Ext2GroupDesc, Ext2Inode, Ext2Superblock, VfsNodeType, EXT2_ROOT_INODE, ROOT_NODE,
};
use crate::drivers::{self, SECTOR_SIZE};
use crate::error::{KResult, KernelError};
use crate::{pr_debug, pr_warn};

pub static mut EXT2_BLOCK_SIZE: u32 = 0;
pub static mut EXT2_INODES_PER_GROUP: u32 = 0;
pub static mut EXT2_INODE_SIZE: u32 = 0;
pub static mut EXT2_BGDT_LBA: u32 = 0;

pub unsafe fn init_ext2_cache(sb: &Ext2Superblock) {
    EXT2_BLOCK_SIZE = 1024 << sb.log_block_size;
    EXT2_INODES_PER_GROUP = sb.inodes_per_group;
    EXT2_INODE_SIZE = if sb.rev_level == 0 {
        128
    } else {
        sb.inode_size as u32
    };

    // BGDT is always the block immediately following the Superblock
    let bgdt_block = sb.first_data_block + 1;
    EXT2_BGDT_LBA = bgdt_block * (EXT2_BLOCK_SIZE / 512);
}

pub fn read_ext2_block(block_number: u32, buffer: &mut [u8]) -> KResult<()> {
    let lba = block_number * 2;
    drivers::read_sectors(lba, 2, buffer)
}

pub fn write_ext2_block(block_number: u32, buffer: &[u8]) -> KResult<()> {
    let lba = block_number * 2;
    if buffer.len() < (2 * SECTOR_SIZE) {
        return Err(KernelError::EINVAL);
    }
    drivers::write_sectors(lba, 2, buffer)
}

pub unsafe fn get_ext2_inode(inode_num: u32) -> KResult<Ext2Inode> {
    if inode_num == 0 {
        return Err(KernelError::EINVAL);
    }

    let group = (inode_num - 1) / EXT2_INODES_PER_GROUP;
    let local_index = (inode_num - 1) % EXT2_INODES_PER_GROUP;

    let bgdt_offset = group * 32;
    let bgdt_lba = EXT2_BGDT_LBA + (bgdt_offset / 512);

    let mut bgdt_buf: [u8; 512] = [0; 512];
    drivers::read_sectors(bgdt_lba, 1, &mut bgdt_buf)?;

    let local_bgdt_offset = (bgdt_offset % 512) as usize;
    let bgd = &*(bgdt_buf.as_ptr().add(local_bgdt_offset) as *const Ext2GroupDesc);

    let inode_table_lba = bgd.bg_inode_table * (EXT2_BLOCK_SIZE / 512);
    let byte_offset = local_index * EXT2_INODE_SIZE;
    let target_lba = inode_table_lba + (byte_offset / 512);
    let sector_offset = (byte_offset % 512) as usize;

    let mut inode_buf: [u8; 512] = [0; 512];
    drivers::read_sectors(target_lba, 1, &mut inode_buf)?;

    let inode = core::ptr::read(inode_buf.as_ptr().add(sector_offset) as *const Ext2Inode);
    Ok(inode)
}

pub unsafe fn parse_directory_block(
    block_buffer: *const u8,
    block_size: usize,
    father_node: *mut super::VfsNode,
    master_node: *mut super::VfsNode,
) {
    let mut offset: usize = 0;
    let mut previous_sibling: *mut super::VfsNode = core::ptr::null_mut();

    while offset < block_size {
        let entry = &*(block_buffer.add(offset) as *const Ext2DirEntry);

        if entry.rec_len == 0 {
            break;
        }

        if entry.inode != 0 {
            let name_ptr = block_buffer.add(offset + 8);
            let name_slice = core::slice::from_raw_parts(name_ptr, entry.name_len as usize);
            let name_str = core::str::from_utf8(name_slice).unwrap_or("?");

            // Skip the "." and ".." relative links to avoid infinite tree recursion
            if name_str != "." && name_str != ".." {
                crate::pr_info!(
                    "Found Entry: {} (Inode: {})\n",
                    name_str,
                    entry.inode as u32
                );

                if let Ok(new_node) = super::alloc_vfs_node() {
                    // Populate the VFS Node
                    core::ptr::copy_nonoverlapping(
                        name_ptr,
                        (*new_node).name.as_mut_ptr(),
                        entry.name_len as usize,
                    );
                    (*new_node).inode = entry.inode;
                    (*new_node).node_type = super::ext2_type_to_vfs(entry.file_type);
                    (*new_node).father = father_node;
                    (*new_node).master = master_node;

                    if let Ok(disk_inode) = get_ext2_inode(entry.inode) {
                        (*new_node).size = disk_inode.i_size;
                        (*new_node).links = disk_inode.i_links_count as u32;
                        (*new_node).rights = disk_inode.i_mode & 0x0FFF; // Extract permission mask
                    } else {
                        pr_warn!(
                            "Failed to retrieve inode {} for entry {}\n",
                            entry.inode as u32,
                            name_str
                        );
                        continue;
                    }

                    if (*father_node).children.is_null() {
                        (*father_node).children = new_node;
                    } else if !previous_sibling.is_null() {
                        (*previous_sibling).next_of_kin = new_node;
                    }

                    previous_sibling = new_node;
                }
            }
        }

        offset += entry.rec_len as usize;
    }
}

pub unsafe fn dump_ext2_sb() -> KResult<()> {
    let mut buffer = [0u8; 1024];
    read_ext2_block(1, &mut buffer)?;

    let superblock: Ext2Superblock = core::ptr::read(buffer.as_ptr() as *const _);

    if superblock.magic != 0xEF53 {
        pr_warn!(
            "Invalid EXT2 superblock magic number: {:#X}",
            superblock.magic as u32
        );
        return Err(KernelError::EINVAL);
    }

    pr_debug!(
        "EXT2 Superblock:\n\
		\tInodes count: {}\n\
		\tBlocks count: {}\n\
		\tFree blocks count: {}\n\
		\tFree inodes count: {}\n\
		\tFirst data block: {}\n\
		\tBlock size: {}\n\
		\tBlocks per group: {}\n\
		\tInodes per group: {}\n",
        superblock.inodes_count as u32,
        superblock.blocks_count as u32,
        superblock.free_blocks_count as u32,
        superblock.free_inodes_count as u32,
        superblock.first_data_block as u32,
        1024 << superblock.log_block_size,
        superblock.blocks_per_group as u32,
        superblock.inodes_per_group as u32,
    );

    let block_size = 1024 << superblock.log_block_size;
    let bgdt_block = superblock.first_data_block + 1;
    let bgdt_lba = bgdt_block * (block_size / 512);

    let mut bgdt_buffer: [u8; 1024] = [0; 1024];
    drivers::read_sectors(bgdt_lba, 2, &mut bgdt_buffer).unwrap();
    let bgdt = &*(bgdt_buffer.as_ptr() as *const Ext2GroupDesc);
    pr_debug!("Inode Table is at Block: {}\n", bgdt.bg_inode_table as u32);

    let inode_table_lba = bgdt.bg_inode_table * (block_size / 512);
    let mut inode_buffer: [u8; 1024] = [0; 1024];
    drivers::read_sectors(inode_table_lba, 2, &mut inode_buffer).unwrap();
    let inode_size = if superblock.rev_level == 0 {
        128
    } else {
        superblock.inode_size as usize
    };
    pr_debug!(
        "Detected Rev Level: {}, Inode Size: {}\n",
        superblock.rev_level as u32,
        inode_size
    );

    let root_inode_offset = (EXT2_ROOT_INODE - 1) as usize * inode_size;
    let root_inode = &*(inode_buffer.as_ptr().add(root_inode_offset) as *const Ext2Inode);

    if (root_inode.i_mode & 0xF000) != 0x4000 {
        pr_warn!(
            "Error: Inode 2 is not a directory. Mode: {:#X}\n",
            root_inode.i_mode as u16
        );
        return Err(KernelError::EINVAL);
    }

    pr_debug!(
        "Root Inode Found! Size: {} bytes, Links: {}\n",
        root_inode.i_size as u32,
        root_inode.i_links_count as u16
    );
    pr_debug!("Root Data Block 0: {}\n", root_inode.i_block[0] as u32);

    init_ext2_cache(&superblock);
    pr_debug!("Reading Root Directory Data Block...\n");

    let root_vfs = super::alloc_vfs_node().unwrap();
    (*root_vfs).name[0] = b'/';
    (*root_vfs).inode = EXT2_ROOT_INODE;
    (*root_vfs).node_type = VfsNodeType::Directory;
    (*root_vfs).master = root_vfs;
    (*root_vfs).father = core::ptr::null_mut();
    (*root_vfs).children = core::ptr::null_mut();
    (*root_vfs).next_of_kin = core::ptr::null_mut();
    (*root_vfs).size = root_inode.i_size;
    (*root_vfs).links = root_inode.i_links_count as u32;
    (*root_vfs).rights = root_inode.i_mode & 0x0FFF;
    ROOT_NODE = root_vfs;
    let dir_block_num = root_inode.i_block[0];
    let dir_lba = dir_block_num * (block_size / 512);
    let mut dir_buffer: [u8; 4096] = [0; 4096];
    drivers::read_sectors(dir_lba, (block_size / 512) as u8, &mut dir_buffer).unwrap();

    parse_directory_block(dir_buffer.as_ptr(), block_size as usize, root_vfs, root_vfs);

    crate::fs::vfs::print_vfs_tree(crate::fs::vfs::ROOT_NODE, 0);

    pr_debug!("VFS Tree Construction Complete.\n");

    Ok(())
}
