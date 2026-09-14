use super::super::{Mount, VfsNode, VfsNodeType};
use super::{Ext2DirEntry, Ext2GroupDesc, Ext2Inode, Ext2Mount, Ext2Superblock, EXT2_ROOT_INODE};
use crate::drivers::{self, BlockDeviceId, SECTOR_SIZE};
use crate::error::{KResult, KernelError};
use crate::fs::{register_mount, vfs, MountPrivate};
use crate::paging::{kfree, kmalloc};
use crate::{pr_debug, pr_err, pr_info, pr_warn};

fn ext2_type_to_vfs(ext2_type: u8) -> VfsNodeType {
    match ext2_type {
        1 => VfsNodeType::File,
        2 => VfsNodeType::Directory,
        7 => VfsNodeType::Symlink,
        _ => VfsNodeType::Unknown,
    }
}

impl Ext2Superblock {
    #[inline(always)]
    pub fn block_size(&self) -> u32 {
        1024 << self.log_block_size
    }
    #[inline(always)]
    pub fn sectors_per_block(&self) -> u32 {
        self.block_size() / 512
    }
    #[inline(always)]
    pub fn inode_size(&self) -> u32 {
        if self.rev_level == 0 {
            128
        } else {
            self.inode_size as u32
        }
    }
    #[inline(always)]
    pub fn bgdt_block(&self) -> u32 {
        self.first_data_block + 1
    }
    #[inline(always)]
    pub fn bgdt_lba(&self) -> u32 {
        self.bgdt_block() * self.sectors_per_block()
    }
    // pub fn set_free_blocks_count(&mut self, count: u32) {
    //     self.free_blocks_count = count;
    // }
}

impl MountPrivate {
    fn as_ext2(&self) -> Option<&Ext2Mount> {
        match self {
            MountPrivate::Ext2(ext2_mount) => Some(ext2_mount),
            _ => None,
        }
    }
}

unsafe fn create_ext2_mount(
    device_id: BlockDeviceId,
    sb: &Ext2Superblock,
) -> KResult<*const Mount> {
    register_mount(|mount_index| Mount {
        device: device_id,
        backend: &super::EXT2_BACKEND,
        private_data: MountPrivate::Ext2(Ext2Mount { device_id, mount_index, sb: *sb }),
    })
}

const DIRECT_BLOCKS: u32 = 12; // 0-11 are direct blocks

unsafe fn ext2_alloc_block(mount: &Ext2Mount) -> KResult<u32> {
    if mount.sb.free_blocks_count == 0 {
        crate::pr_err!("Ext2: No free blocks remaining on disk.\n");
        return Err(KernelError::ENOSPC); // Use EIO if ENOSPC is not defined
    }

    let bgdt_alloc = kmalloc(512)?;
    let bgdt_buf: &mut [u8; 512] = &mut *(bgdt_alloc as *mut [u8; 512]);
    drivers::read_sectors(mount.device_id, mount.sb.bgdt_lba(), 1, bgdt_buf)?;
    let bgd = &mut *(bgdt_buf.as_mut_ptr() as *mut Ext2GroupDesc);

    if bgd.bg_free_blocks_count == 0 {
        // multiple block groups
        crate::pr_err!("Ext2: No free blocks remaining in Block Group 0.\n");
        return Err(KernelError::ENOSPC);
    }

    let bitmap_block = bgd.bg_block_bitmap;
    let bitmap_alloc = kmalloc(4096)?;
    let bitmap_ptr: &mut [u8; 4096] = &mut *(bitmap_alloc as *mut [u8; 4096]);
    read_block(mount, bitmap_block, bitmap_ptr)?;

    let block_size_bytes = mount.sb.block_size() as usize;
    let mut allocated_bit = 0;
    let mut found = false;

    for byte_idx in 0..block_size_bytes {
        if bitmap_ptr[byte_idx] != 0xFF {
            for bit_idx in 0..8 {
                if (bitmap_ptr[byte_idx] & (1 << bit_idx)) == 0 {
                    // Found a free bit! Set it to 1
                    bitmap_ptr[byte_idx] |= 1 << bit_idx;
                    allocated_bit = (byte_idx * 8) + bit_idx;
                    found = true;
                    break;
                }
            }
        }
        if found {
            break;
        }
    }

    if !found {
        return Err(KernelError::ENOSPC);
    }

    write_block(mount, bitmap_block, &bitmap_ptr[..block_size_bytes])?;

    bgd.bg_free_blocks_count -= 1;
    drivers::write_sectors(mount.device_id, mount.sb.bgdt_lba(), 1, bgdt_buf)?;
    kfree(bitmap_alloc).ok();
    kfree(bgdt_alloc).ok();

    // mount.sb.set_free_blocks_count(mount.sb.free_blocks_count - 1);

    let block_num = mount.sb.first_data_block + allocated_bit as u32;
    Ok(block_num)
}

unsafe fn ext2_free_block(mount: &Ext2Mount, block_num: u32) -> KResult<()> {
    if block_num == 0 {
        return Ok(()); // Block 0 is never a valid data block to free
    }

    let sb_alloc = kmalloc(1024)?;
    let sb_buf: &mut [u8; 1024] = &mut *(sb_alloc as *mut [u8; 1024]);
    drivers::read_sectors(mount.device_id, 2, 2, sb_buf)?;
    let sb = &mut *(sb_buf.as_mut_ptr() as *mut Ext2Superblock);

    if block_num < sb.first_data_block || block_num >= sb.blocks_count {
        crate::pr_err!(
            "Ext2: Attempted to free out-of-bounds block {}\n",
            block_num
        );
        return Err(KernelError::EINVAL);
    }

    let group = (block_num - sb.first_data_block) / sb.blocks_per_group;
    let local_bit_index = (block_num - sb.first_data_block) % sb.blocks_per_group;

    if group > 0 {
        unimplemented!("Ext2: Multiple block groups not yet supported for freeing");
    }

    let bgdt_alloc = kmalloc(512)?;
    let bgdt_buf: &mut [u8; 512] = &mut *(bgdt_alloc as *mut [u8; 512]);
    drivers::read_sectors(mount.device_id, mount.sb.bgdt_lba(), 1, bgdt_buf)?;
    let bgd = &mut *(bgdt_buf.as_mut_ptr() as *mut Ext2GroupDesc);

    let bitmap_block = bgd.bg_block_bitmap;
    let bitmap_alloc = kmalloc(4096)?;
    let bitmap_ptr: &mut [u8; 4096] = &mut *(bitmap_alloc as *mut [u8; 4096]);
    read_block(mount, bitmap_block, bitmap_ptr)?;

    let byte_idx = (local_bit_index / 8) as usize;
    let bit_offset = local_bit_index % 8;

    if (bitmap_ptr[byte_idx] & (1 << bit_offset)) == 0 {
        crate::pr_warn!("Ext2: Double free detected for block {}\n", block_num);
        return Ok(());
    }

    bitmap_ptr[byte_idx] &= !(1 << bit_offset);

    // 6. Write Bitmap back to disk
    write_block(
        mount,
        bitmap_block,
        &bitmap_ptr[..(mount.sb.block_size() as usize)],
    )?;

    // 7. Update and write BGDT
    bgd.bg_free_blocks_count += 1;
    drivers::write_sectors(mount.device_id, mount.sb.bgdt_lba(), 1, bgdt_buf)?;
    kfree(bgdt_alloc).ok();
    kfree(bitmap_alloc).ok();
    kfree(sb_alloc).ok();

    let sb_mut = (&mount.sb as *const Ext2Superblock as *mut Ext2Superblock)
        .as_mut()
        .unwrap();
    sb_mut.free_blocks_count += 1;

    Ok(())
}

unsafe fn allocate_zeroed_block(mount: &Ext2Mount) -> KResult<u32> {
    let new_block = ext2_alloc_block(mount)?;

    let zero_buf = [0u8; 4096];

    // Only zero out the exact bytes required by the dynamic FS block size
    write_block(
        mount,
        new_block,
        &zero_buf[..(mount.sb.block_size() as usize)],
    )?;
    pr_err!("Ext2: Failed to zero new block");

    Ok(new_block)
}

unsafe fn free_block_tree(mount: &Ext2Mount, block_num: u32, depth: u8) -> KResult<()> {
    if block_num == 0 {
        return Ok(());
    }

    if depth == 0 {
        return ext2_free_block(mount, block_num);
    }

    let mut block_buf = [0u8; 4096];
    read_block(mount, block_num, &mut block_buf)?;

    let pointers =
        core::slice::from_raw_parts(block_buf.as_ptr() as *const u32, block_buf.len() / 4);
    for &child_block in pointers {
        if child_block != 0 {
            free_block_tree(mount, child_block, depth - 1)?;
        }
    }

    ext2_free_block(mount, block_num)
}

unsafe fn get_alloc_indirect(
    mount: &Ext2Mount,
    indirect_block_num: u32,
    pointer_index: u32,
    inode_num: u32,
    inode: &mut Ext2Inode,
    sectors_per_block: u32,
) -> KResult<u32> {
    let mut block_buf = [0u8; 4096];
    read_block(mount, indirect_block_num, &mut block_buf)?;

    // Cast the byte buffer to a u32 slice to read the pointers
    let pointers = unsafe {
        core::slice::from_raw_parts_mut(block_buf.as_mut_ptr() as *mut u32, block_buf.len() / 4)
    };

    let mut target_phys_block = pointers[pointer_index as usize];

    if target_phys_block == 0 {
        target_phys_block = allocate_zeroed_block(mount)?;
        pointers[pointer_index as usize] = target_phys_block;

        // Write the updated pointer block back to disk
        let Ok(()) = write_block(mount, indirect_block_num, &block_buf) else {
            ext2_free_block(mount, target_phys_block).ok(); // Attempt to free
            return Err(KernelError::EIO);
        };

        // Update the main inode's sector count to reflect the new data block
        inode.i_blocks += sectors_per_block;
        let Ok(()) = update_inode(mount, inode_num, inode) else {
            ext2_free_block(mount, target_phys_block).ok();
            return Err(KernelError::EIO);
        };
    }

    Ok(target_phys_block)
}

unsafe fn get_or_allocate_physical_block(
    mount: &Ext2Mount,
    inode_num: u32,
    logical_block: u32,
) -> KResult<u32> {
    let mut inode = get_inode(mount, inode_num)?;

    // Ext2 i_blocks field is ALWAYS measured in 512-byte sectors,
    let pointers_per_block = mount.sb.block_size() / 4;

    // --------------------------------------------------------
    // SCENARIO 1: Direct Blocks (0 to 11)
    // --------------------------------------------------------
    if logical_block < DIRECT_BLOCKS {
        let mut phys_block = inode.i_block[logical_block as usize];

        if phys_block == 0 {
            phys_block = allocate_zeroed_block(mount)?;
            inode.i_block[logical_block as usize] = phys_block;
            inode.i_blocks += mount.sb.sectors_per_block();
            let Ok(()) = update_inode(mount, inode_num, &inode) else {
                ext2_free_block(mount, phys_block).ok();
                return Err(KernelError::EIO);
            };
        }

        return Ok(phys_block);
    }

    let mut logical_offset = logical_block - DIRECT_BLOCKS;

    // --------------------------------------------------------
    // SCENARIO 2: Singly Indirect Block (12)
    // --------------------------------------------------------
    if logical_offset < pointers_per_block {
        let mut indirect_block = inode.i_block[12];

        // Allocate the indirect block itself if it doesn't exist
        if indirect_block == 0 {
            indirect_block = allocate_zeroed_block(mount)?;
            inode.i_block[12] = indirect_block;
            inode.i_blocks += mount.sb.sectors_per_block();
            let Ok(()) = update_inode(mount, inode_num, &inode) else {
                ext2_free_block(mount, indirect_block).ok();
                return Err(KernelError::EIO);
            };
        }

        return get_alloc_indirect(
            mount,
            indirect_block,
            logical_offset,
            inode_num,
            &mut inode,
            mount.sb.sectors_per_block(),
        );
    }

    logical_offset -= pointers_per_block;

    // --------------------------------------------------------
    // SCENARIO 3: Doubly Indirect Block (13)
    // --------------------------------------------------------
    let doubly_limit = pointers_per_block * pointers_per_block;
    if logical_offset < doubly_limit {
        pr_err!("Ext2: Doubly indirect block allocation not yet supported");
    } else {
        pr_err!("Ext2: Triply indirect block allocation not yet supported");
    }

    Err(KernelError::ENOSYS)
}

fn read_block(mount: &Ext2Mount, block_number: u32, buffer: &mut [u8]) -> KResult<()> {
    let lba = block_number * 2;
    drivers::read_sectors(mount.device_id, lba, 2, buffer)
}

fn write_block(mount: &Ext2Mount, block_number: u32, buffer: &[u8]) -> KResult<()> {
    let lba = block_number * 2;
    if buffer.len() < (2 * SECTOR_SIZE) {
        return Err(KernelError::EINVAL);
    }
    drivers::write_sectors(mount.device_id, lba, 2, buffer)
}

unsafe fn get_inode(mount: &Ext2Mount, inode_num: u32) -> KResult<Ext2Inode> {
    if inode_num == 0 {
        return Err(KernelError::EINVAL);
    }

    let group = (inode_num - 1) / mount.sb.inodes_per_group;
    let local_index = (inode_num - 1) % mount.sb.inodes_per_group;

    let bgdt_offset = group * 32;
    let bgdt_lba = mount.sb.bgdt_lba() + (bgdt_offset / 512);

    let mut bgdt_buf: [u8; 512] = [0; 512];
    drivers::read_sectors(mount.device_id, bgdt_lba, 1, &mut bgdt_buf)?;

    let local_bgdt_offset = (bgdt_offset % 512) as usize;
    let bgd = &*(bgdt_buf.as_ptr().add(local_bgdt_offset) as *const Ext2GroupDesc);

    let inode_table_lba = bgd.bg_inode_table * (mount.sb.block_size() / 512);
    let byte_offset = local_index * mount.sb.inode_size();
    let target_lba = inode_table_lba + (byte_offset / 512);
    let sector_offset = (byte_offset % 512) as usize;

    let mut inode_buf: [u8; 512] = [0; 512];
    drivers::read_sectors(mount.device_id, target_lba, 1, &mut inode_buf)?;

    let inode = core::ptr::read(inode_buf.as_ptr().add(sector_offset) as *const Ext2Inode);

    Ok(inode)
}

unsafe fn update_inode(mount: &Ext2Mount, inode_num: u32, inode: &Ext2Inode) -> KResult<()> {
    if inode_num == 0 {
        return Err(KernelError::EINVAL);
    }

    let group = (inode_num - 1) / mount.sb.inodes_per_group;
    let local_index = (inode_num - 1) % mount.sb.inodes_per_group;

    let bgdt_offset = group * 32;
    let bgdt_lba = mount.sb.bgdt_lba() + (bgdt_offset / 512);

    let mut bgdt_buf: [u8; 512] = [0; 512];
    drivers::read_sectors(mount.device_id, bgdt_lba, 1, &mut bgdt_buf)?;

    let local_bgdt_offset = (bgdt_offset % 512) as usize;
    let bgd = &*(bgdt_buf.as_ptr().add(local_bgdt_offset) as *const Ext2GroupDesc);

    let inode_table_lba = bgd.bg_inode_table * (mount.sb.block_size() / 512);
    let byte_offset = local_index * mount.sb.inode_size();
    let target_lba = inode_table_lba + (byte_offset / 512);
    let sector_offset = (byte_offset % 512) as usize;

    let mut inode_buf: [u8; 512] = [0; 512];
    drivers::read_sectors(mount.device_id, target_lba, 1, &mut inode_buf)?;

    core::ptr::write(
        inode_buf.as_mut_ptr().add(sector_offset) as *mut Ext2Inode,
        *inode,
    );

    drivers::write_sectors(mount.device_id, target_lba, 1, &inode_buf)?;

    Ok(())
}

unsafe fn parse_directory_block(
    mount: &Ext2Mount,
    block_buffer: *const u8,
    block_size: usize,
    father_node: *mut VfsNode,
    master_node: *mut VfsNode,
) {
    let mut offset: usize = 0;
    let mut previous_sibling: *mut VfsNode = core::ptr::null_mut();

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

                if let Ok(new_node) = vfs::alloc_vfs_node() {
                    // Populate the VFS Node
                    core::ptr::copy_nonoverlapping(
                        name_ptr,
                        (*new_node).name.as_mut_ptr(),
                        entry.name_len as usize,
                    );
                    (*new_node).inode = entry.inode;
                    (*new_node).node_type = ext2_type_to_vfs(entry.file_type);
                    (*new_node).father = father_node;
                    (*new_node).master = master_node;

                    if let Ok(disk_inode) = get_inode(mount, entry.inode) {
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

// TODO: safeguard all `.unwrap()` calls with rollback in case of failure
#[unsafe(no_mangle)]
pub unsafe fn mount_device(device_id: BlockDeviceId) -> KResult<()> {
    mount_device_at(device_id, core::ptr::null_mut())
}

pub unsafe fn mount_device_at(device_id: BlockDeviceId, target: *mut VfsNode) -> KResult<()> {
    if !target.is_null() {
        if (*target).node_type != VfsNodeType::Directory {
            return Err(KernelError::ENOTDIR);
        }
        if !(*target).children.is_null() {
            return Err(KernelError::EBUSY);
        }
    }

    pr_info!("Mounting EXT2 filesystem on device {}\n", device_id);
    let mut buffer = [0u8; 1024];
    let Ok(()) = drivers::read_sectors(device_id, 2, 2, &mut buffer) else {
        pr_warn!("Failed to read EXT2 superblock from device {}\n", device_id);
        return Err(KernelError::EIO);
    };

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

    let mount_wrapper = create_ext2_mount(device_id, &superblock)?;
    let mount = (*mount_wrapper).private_data.as_ext2().unwrap();

    let mut bgdt_buffer: [u8; 1024] = [0; 1024];
    drivers::read_sectors(device_id, mount.sb.bgdt_lba(), 2, &mut bgdt_buffer).unwrap();
    let bgdt = &*(bgdt_buffer.as_ptr() as *const Ext2GroupDesc);
    pr_debug!("Inode Table is at Block: {}\n", bgdt.bg_inode_table as u32);

    let inode_table_lba = bgdt.bg_inode_table * (mount.sb.block_size() / 512);
    let mut inode_buffer: [u8; 1024] = [0; 1024];
    drivers::read_sectors(device_id, inode_table_lba, 2, &mut inode_buffer).unwrap();

    let root_inode_offset = (EXT2_ROOT_INODE - 1) * mount.sb.inode_size();
    let root_inode = &*(inode_buffer.as_ptr().add(root_inode_offset as usize) as *const Ext2Inode);

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

    pr_debug!("Reading Root Directory Data Block...\n");

    let root_vfs = vfs::alloc_vfs_node().unwrap();
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
    (*root_vfs).mount = mount_wrapper;
    if target.is_null() {
        vfs::ROOT_NODE = root_vfs;
    }
    let dir_block_num = root_inode.i_block[0];
    let dir_lba = dir_block_num * (mount.sb.block_size() / 512);
    let mut dir_buffer: [u8; 4096] = [0; 4096];
    drivers::read_sectors(
        mount.device_id,
        dir_lba,
        (mount.sb.block_size() / 512) as u8,
        &mut dir_buffer,
    )
    .unwrap();

    parse_directory_block(
        &mount,
        dir_buffer.as_ptr(),
        mount.sb.block_size() as usize,
        root_vfs,
        root_vfs,
    );

    if !target.is_null() {
        (*target).children = (*root_vfs).children;
        let mut child = (*target).children;
        while !child.is_null() {
            (*child).father = target;
            child = (*child).next_of_kin;
        }
        vfs::mount_node(target, root_vfs)?;
    }

    let dev_dir = vfs::alloc_vfs_node().unwrap();
    (*dev_dir).name[0] = b'd';
    (*dev_dir).name[1] = b'e';
    (*dev_dir).name[2] = b'v';
    (*dev_dir).inode = 0;
    (*dev_dir).node_type = VfsNodeType::Directory;
    (*dev_dir).master = root_vfs;
    (*dev_dir).children = core::ptr::null_mut();
    (*dev_dir).next_of_kin = core::ptr::null_mut();
    (*dev_dir).size = 0;
    (*dev_dir).links = 1;
    (*dev_dir).rights = 0o755;
    vfs::append_child(root_vfs, dev_dir);

    crate::fs::vfs::print_vfs_tree(crate::fs::vfs::ROOT_NODE, 0);

    pr_debug!("VFS Tree Construction Complete.\n");

    Ok(())
}

pub unsafe fn read_from_inode(
    mount_wrap: *const Mount,
    inode_num: u32,
    buffer: &mut [u8],
    offset: u32,
) -> KResult<usize> {
    let mount = &*(mount_wrap as *const Ext2Mount);
    let inode = get_inode(mount, inode_num)?;

    let file_size = inode.i_size;
    if offset >= file_size {
        return Ok(0); // EOF
    }

    let bytes_to_read = core::cmp::min(buffer.len() as u32, file_size - offset);

    let logical_block_idx = (offset / mount.sb.block_size()) as usize;
    let offset_in_block = (offset % mount.sb.block_size()) as usize;

    if logical_block_idx > 11 {
        pr_err!("File too large, indirect blocks not implemented.\n");
        return Err(KernelError::ENOSYS);
    }

    let physical_block = inode.i_block[logical_block_idx];
    if physical_block == 0 {
        return Ok(0); // Sparse file, treat as EOF
    }

    let mut block_buf: [u8; 1024] = [0; 1024];
    if read_block(mount, physical_block, &mut block_buf).is_err() {
        return Err(KernelError::EIO);
    }

    core::ptr::copy_nonoverlapping(
        block_buf.as_ptr().add(offset_in_block),
        buffer.as_mut_ptr(),
        bytes_to_read as usize,
    );

    Ok(bytes_to_read as usize)
}

pub unsafe fn write_to_inode(
    mount_wrap: *const Mount,
    inode_num: u32,
    buffer: &[u8],
    mut offset: u32,
) -> KResult<usize> {
    let mount = &*(mount_wrap as *const Ext2Mount);
    let mut bytes_written = 0;
    let mut remaining = buffer.len();

    while remaining > 0 {
        let logical_block = offset / mount.sb.block_size();
        let block_offset = offset % mount.sb.block_size();
        let space_in_block = mount.sb.block_size() - block_offset;
        let write_size = core::cmp::min(remaining, space_in_block as usize);
        let phys_block = get_or_allocate_physical_block(mount, inode_num, logical_block)?;

        let mut block_buf = [0u8; 4096];

        // Read-Modify-Write
        if write_size < mount.sb.block_size() as usize {
            read_block(mount, phys_block, &mut block_buf)?;
        }

        let src_start = bytes_written;
        let src_end = src_start + write_size;
        let dst_start = block_offset as usize;
        let dst_end = dst_start + write_size;

        block_buf[dst_start..dst_end].copy_from_slice(&buffer[src_start..src_end]);

        write_block(mount, phys_block, &block_buf)?;

        bytes_written += write_size;
        remaining -= write_size;
        offset += write_size as u32;
    }

    Ok(bytes_written)
}

pub unsafe fn truncate_inode(mount_wrap: *const Mount, inode_num: u32) -> KResult<()> {
    let mount = &*(mount_wrap as *const Ext2Mount);
    let mut inode = get_inode(mount, inode_num)?;

    for block_index in 0..12 {
        let block_num = inode.i_block[block_index];
        if block_num != 0 {
            ext2_free_block(mount, block_num)?;
            inode.i_block[block_index] = 0;
        }
    }

    if inode.i_block[12] != 0 {
        free_block_tree(mount, inode.i_block[12], 1)?;
        inode.i_block[12] = 0;
    }

    if inode.i_block[13] != 0 {
        free_block_tree(mount, inode.i_block[13], 2)?;
        inode.i_block[13] = 0;
    }

    if inode.i_block[14] != 0 {
        free_block_tree(mount, inode.i_block[14], 3)?;
        inode.i_block[14] = 0;
    }

    inode.i_size = 0;
    inode.i_blocks = 0;
    update_inode(mount, inode_num, &inode)
}

pub unsafe fn lazy_load_directory(mount_wrap: *const Mount, dir_node_num: u32) -> KResult<()> {
    let mount = &*(mount_wrap as *const Ext2Mount);
    let inode = get_inode(mount, dir_node_num)?;
    // For simplicity, we only read the first direct block of the directory.
    // Large directories require reading i_block[1], i_block[2], etc.
    let block_num = inode.i_block[0];
    if block_num == 0 {
        return Ok(());
    }

    let lba = block_num * (mount.sb.block_size() / 512);
    let sector_count = (mount.sb.block_size() / 512) as u8;

    let mut dir_buf: [u8; 4096] = [0; 4096];
    drivers::read_sectors(mount.device_id, lba, sector_count, &mut dir_buf)?;

    Ok(())
}
