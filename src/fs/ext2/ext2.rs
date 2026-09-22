use super::super::{Mount, VfsNode, VfsNodeType};
use super::{Ext2DirEntry, Ext2GroupDesc, Ext2Inode, Ext2Mount, Ext2Superblock, EXT2_ROOT_INODE};
use crate::drivers::{self, BlockDeviceId};
use crate::error::{KResult, KernelError};
use crate::fs::{register_mount, vfs, MountPrivate};
use crate::paging::{kfree, kmalloc, HeapBuffer};
use crate::{pr_debug, pr_err, pr_info, pr_warn};
use core::convert::TryInto;

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

unsafe fn ext2_mount_from_wrapper(mount_wrapper: *const Mount) -> KResult<Ext2Mount> {
    match (*mount_wrapper).private_data {
        MountPrivate::Ext2(mount) => Ok(mount),
        MountPrivate::Raw => Err(KernelError::EOPNOTSUPP),
    }
}

const DIRECT_BLOCKS: u32 = 12; // 0-11 are direct blocks

unsafe fn ext2_alloc_block(mount: &Ext2Mount) -> KResult<u32> {
    if mount.sb.free_blocks_count == 0 {
        crate::pr_err!("Ext2: No free blocks remaining on disk.\n");
        return Err(KernelError::ENOSPC); // Use EIO if ENOSPC is not defined
    }

    let mut bgdt_buf = HeapBuffer::new(512)?;
    drivers::read_sectors(mount.device_id, mount.sb.bgdt_lba(), 1, &mut bgdt_buf)?;
    let bgd = &mut *(bgdt_buf.as_mut_ptr() as *mut Ext2GroupDesc);

    if bgd.bg_free_blocks_count == 0 {
        // multiple block groups
        crate::pr_err!("Ext2: No free blocks remaining in Block Group 0.\n");
        return Err(KernelError::ENOSPC);
    }

    let bitmap_block = bgd.bg_block_bitmap;
    let mut bitmap_buf = HeapBuffer::new(4096)?;
    read_block(mount, bitmap_block, &mut bitmap_buf)?;

    let block_size_bytes = mount.sb.block_size() as usize;
    let mut allocated_bit = 0;
    let mut found = false;

    for byte_idx in 0..block_size_bytes {
        if bitmap_buf[byte_idx] != 0xFF {
            for bit_idx in 0..8 {
                if (bitmap_buf[byte_idx] & (1 << bit_idx)) == 0 {
                    // Found a free bit! Set it to 1
                    bitmap_buf[byte_idx] |= 1 << bit_idx;
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

    write_block(mount, bitmap_block, &bitmap_buf[..block_size_bytes])?;

    bgd.bg_free_blocks_count -= 1;
    drivers::write_sectors(mount.device_id, mount.sb.bgdt_lba(), 1, &bgdt_buf)?;

    let block_num = mount.sb.first_data_block + allocated_bit as u32;
    Ok(block_num)
}

unsafe fn ext2_alloc_inode(mount: &Ext2Mount) -> KResult<u32> {
    if mount.sb.free_inodes_count == 0 {
        return Err(KernelError::ENOSPC);
    }

    let mut bgdt_buf = [0u8; 512];
    drivers::read_sectors(mount.device_id, mount.sb.bgdt_lba(), 1, &mut bgdt_buf)?;
    let bgd = &mut *(bgdt_buf.as_mut_ptr() as *mut Ext2GroupDesc);
    if bgd.bg_free_inodes_count == 0 {
        return Err(KernelError::ENOSPC);
    }

    let mut bitmap_buf = HeapBuffer::new(4096)?;
    read_block(mount, bgd.bg_inode_bitmap, &mut bitmap_buf)?;
    let inode_limit = core::cmp::min(mount.sb.inodes_per_group as usize, bitmap_buf.len() * 8);
    let first_inode = if mount.sb.rev_level == 0 {
        0
    } else {
        mount.sb.first_ino.saturating_sub(1) as usize
    };
    let mut selected = None;
    for bit in first_inode..inode_limit {
        if bitmap_buf[bit / 8] & (1 << (bit % 8)) == 0 {
            bitmap_buf[bit / 8] |= 1 << (bit % 8);
            selected = Some(bit);
            break;
        }
    }
    let bit = selected.ok_or(KernelError::ENOSPC)?;
    write_block(mount, bgd.bg_inode_bitmap, &bitmap_buf)?;
    let inode_number = bit as u32 + 1;

    bgd.bg_free_inodes_count -= 1;
    drivers::write_sectors(mount.device_id, mount.sb.bgdt_lba(), 1, &bgdt_buf)?;

    let mut sb_buf = [0u8; 1024];
    drivers::read_sectors(mount.device_id, 2, 2, &mut sb_buf)?;
    let sb = &mut *(sb_buf.as_mut_ptr() as *mut Ext2Superblock);
    sb.free_inodes_count -= 1;
    drivers::write_sectors(mount.device_id, 2, 2, &sb_buf)?;
    Ok(inode_number)
}

unsafe fn ext2_free_block(mount: &Ext2Mount, block_num: u32) -> KResult<()> {
    if block_num == 0 {
        return Ok(()); // Block 0 is never a valid data block to free
    }

    let mut sb_buf = HeapBuffer::new(1024)?;
    drivers::read_sectors(mount.device_id, 2, 2, &mut sb_buf)?;
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
        crate::pr_err!("Ext2: Multiple block groups not yet supported for freeing\n");
        return Err(KernelError::EOPNOTSUPP);
    }

    let mut bgdt_buf = HeapBuffer::new(512)?;
    drivers::read_sectors(mount.device_id, mount.sb.bgdt_lba(), 1, &mut bgdt_buf)?;
    let bgd = &mut *(bgdt_buf.as_mut_ptr() as *mut Ext2GroupDesc);

    let bitmap_block = bgd.bg_block_bitmap;
    let mut bitmap_buf = HeapBuffer::new(4096)?;
    read_block(mount, bitmap_block, &mut bitmap_buf)?;

    let byte_idx = (local_bit_index / 8) as usize;
    let bit_offset = local_bit_index % 8;

    if (bitmap_buf[byte_idx] & (1 << bit_offset)) == 0 {
        crate::pr_warn!("Ext2: Double free detected for block {}\n", block_num);
        return Ok(());
    }

    bitmap_buf[byte_idx] &= !(1 << bit_offset);

    // 6. Write Bitmap back to disk
    write_block(
        mount,
        bitmap_block,
        &bitmap_buf[..(mount.sb.block_size() as usize)],
    )?;

    // 7. Update and write BGDT
    bgd.bg_free_blocks_count += 1;
    drivers::write_sectors(mount.device_id, mount.sb.bgdt_lba(), 1, &bgdt_buf)?;

    // 8. Persist the incremented free-block count to the on-disk superblock too
    sb.free_blocks_count += 1;
    drivers::write_sectors(mount.device_id, 2, 2, &sb_buf)?;

    Ok(())
}

unsafe fn allocate_zeroed_block(mount: &Ext2Mount) -> KResult<u32> {
    let new_block = ext2_alloc_block(mount)?;

    let mut zero_buf = HeapBuffer::new(4096)?;
    zero_buf.fill(0);
    write_block(
        mount,
        new_block,
        &zero_buf[..(mount.sb.block_size() as usize)],
    )?;
    pr_debug!("Ext2: zeroed new block {}\n", new_block);

    Ok(new_block)
}

unsafe fn free_block_tree(mount: &Ext2Mount, block_num: u32, depth: u8) -> KResult<()> {
    if block_num == 0 {
        return Ok(());
    }

    if depth == 0 {
        return ext2_free_block(mount, block_num);
    }

    let mut block_buf = HeapBuffer::new(4096)?;
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
    let mut block_buf = HeapBuffer::new(4096)?;
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
        if let Err(e) = write_block(mount, indirect_block_num, &block_buf) {
            ext2_free_block(mount, target_phys_block).ok();
            return Err(e);
        }

        // Update the main inode's sector count to reflect the new data block
        inode.i_blocks += sectors_per_block;
        if let Err(e) = update_inode(mount, inode_num, inode) {
            ext2_free_block(mount, target_phys_block).ok();
            return Err(e);
        }
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

/// FIX: previously hardcoded `block_number * 2` / count = 2, which only ever
/// reads/writes the first 1024 bytes of a block.
fn read_block(mount: &Ext2Mount, block_number: u32, buffer: &mut [u8]) -> KResult<()> {
    let sectors = mount.sb.sectors_per_block();
    let block_size = mount.sb.block_size() as usize;
    if buffer.len() < block_size {
        return Err(KernelError::EINVAL);
    }
    let lba = block_number * sectors;
    drivers::read_sectors(
        mount.device_id,
        lba,
        sectors as u8,
        &mut buffer[..block_size],
    )
}

fn write_block(mount: &Ext2Mount, block_number: u32, buffer: &[u8]) -> KResult<()> {
    let sectors = mount.sb.sectors_per_block();
    let block_size = mount.sb.block_size() as usize;
    if buffer.len() < block_size {
        return Err(KernelError::EINVAL);
    }
    let lba = block_number * sectors;
    drivers::write_sectors(mount.device_id, lba, sectors as u8, &buffer[..block_size])
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

unsafe fn add_directory_entry(
    mount: &Ext2Mount,
    parent_inode: u32,
    child_inode: u32,
    name: &str,
    node_type: VfsNodeType,
) -> KResult<()> {
    let mut parent = get_inode(mount, parent_inode)?;
    if parent.i_mode & 0xF000 != 0x4000 {
        return Err(KernelError::ENOTDIR);
    }
    let new_block = parent.i_block[0] == 0;
    let block_number = if new_block {
        let block = allocate_zeroed_block(mount)?;
        parent.i_block[0] = block;
        parent.i_blocks += mount.sb.sectors_per_block();
        update_inode(mount, parent_inode, &parent)?;
        block
    } else {
        parent.i_block[0]
    };

    let block_size = mount.sb.block_size() as usize;
    let block_alloc = kmalloc(4096)?;
    let block = &mut *(block_alloc as *mut [u8; 4096]);
    let result = (|| {
        read_block(mount, block_number, block)?;
        if new_block {
            block[4..6].copy_from_slice(&(block_size as u16).to_le_bytes());
        }
        let name_bytes = name.as_bytes();
        let needed = (8 + name_bytes.len() + 3) & !3;
        let mut offset = 0usize;

        while offset + 8 <= block_size {
            let inode = u32::from_le_bytes(block[offset..offset + 4].try_into().unwrap());
            let record_length =
                u16::from_le_bytes(block[offset + 4..offset + 6].try_into().unwrap()) as usize;
            let name_length = block[offset + 6] as usize;
            if record_length < 8 || offset + record_length > block_size {
                return Err(KernelError::EIO);
            }
            let minimum = (8 + name_length + 3) & !3;
            if inode == 0 && record_length >= needed {
                write_directory_record(
                    block,
                    offset,
                    record_length,
                    child_inode,
                    name_bytes,
                    node_type,
                );
                write_block(mount, block_number, block)?;
                return Ok(());
            }
            if offset + record_length == block_size && record_length >= minimum + needed {
                block[offset + 4..offset + 6].copy_from_slice(&(minimum as u16).to_le_bytes());
                write_directory_record(
                    block,
                    offset + minimum,
                    record_length - minimum,
                    child_inode,
                    name_bytes,
                    node_type,
                );
                write_block(mount, block_number, block)?;
                return Ok(());
            }
            offset += record_length;
        }
        Err(KernelError::ENOSPC)
    })();
    kfree(block_alloc).ok();
    result
}

unsafe fn write_directory_record(
    block: &mut [u8],
    offset: usize,
    record_length: usize,
    inode: u32,
    name: &[u8],
    node_type: VfsNodeType,
) {
    block[offset..offset + 4].copy_from_slice(&inode.to_le_bytes());
    block[offset + 4..offset + 6].copy_from_slice(&(record_length as u16).to_le_bytes());
    block[offset + 6] = name.len() as u8;
    block[offset + 7] = match node_type {
        VfsNodeType::Directory => 2,
        _ => 1,
    };
    block[offset + 8..offset + 8 + name.len()].copy_from_slice(name);
}

unsafe fn init_new_directory(
    mount: &Ext2Mount,
    parent_inode: u32,
    child_inode: u32,
) -> KResult<()> {
    let block_size = mount.sb.block_size() as usize;
    let new_block = allocate_zeroed_block(mount)?;

    let block_alloc = kmalloc(4096)?;
    let block = &mut *(block_alloc as *mut [u8; 4096]);
    block.fill(0);

    let result = (|| {
        // "." -> self
        let dot_len = (8 + 1 + 3) & !3;
        write_directory_record(block, 0, dot_len, child_inode, b".", VfsNodeType::Directory);

        // ".." -> parent, consumes the rest of the block
        write_directory_record(
            block,
            dot_len,
            block_size - dot_len,
            parent_inode,
            b"..",
            VfsNodeType::Directory,
        );

        write_block(mount, new_block, &block[..block_size])
    })();
    kfree(block_alloc).ok();
    if let Err(error) = result {
        ext2_free_block(mount, new_block).ok();
        return Err(error);
    }

    let mut child = get_inode(mount, child_inode)?;
    child.i_block[0] = new_block;
    child.i_size = block_size as u32;
    child.i_blocks += mount.sb.sectors_per_block();
    update_inode(mount, child_inode, &child)?;

    // The child's ".." entry is a new link to the parent directory.
    let mut parent = get_inode(mount, parent_inode)?;
    parent.i_links_count += 1;
    update_inode(mount, parent_inode, &parent)
}

pub(crate) unsafe fn create_node(
    mount_wrapper: *const Mount,
    parent_inode: u32,
    name: &str,
    node_type: VfsNodeType,
    mode: u16,
    uid: u32,
    gid: u32,
) -> KResult<u32> {
    if name.is_empty() || name.len() > 255 {
        return Err(KernelError::ENAMETOOLONG);
    }
    let mount = match (*mount_wrapper).private_data {
        MountPrivate::Ext2(mount) => mount,
        MountPrivate::Raw => return Err(KernelError::EOPNOTSUPP),
    };
    let inode_number = ext2_alloc_inode(&mount)?;
    let file_type = match node_type {
        VfsNodeType::Directory => 0x4000,
        VfsNodeType::File => 0x8000,
        _ => return Err(KernelError::EOPNOTSUPP),
    };
    let inode = Ext2Inode {
        i_mode: file_type | (mode & 0o777),
        i_uid: uid as u16,
        i_size: 0,
        i_atime: 0,
        i_ctime: 0,
        i_mtime: 0,
        i_dtime: 0,
        i_gid: gid as u16,
        i_links_count: if node_type == VfsNodeType::Directory {
            2
        } else {
            1
        },
        i_blocks: 0,
        i_flags: 0,
        i_osd1: 0,
        i_block: [0; 15],
        i_generation: 0,
        i_file_acl: 0,
        i_dir_acl: 0,
        i_faddr: 0,
        i_osd2: [0; 3],
    };
    if let Err(error) = update_inode(&mount, inode_number, &inode) {
        return Err(error);
    }
    if let Err(error) = add_directory_entry(&mount, parent_inode, inode_number, name, node_type) {
        return Err(error);
    }
    if node_type == VfsNodeType::Directory {
        if let Err(error) = init_new_directory(&mount, parent_inode, inode_number) {
            return Err(error);
        }
    }
    Ok(inode_number)
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
                        (*new_node).owner_uid = disk_inode.i_uid as u32;
                        (*new_node).owner_gid = disk_inode.i_gid as u32;
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
    (*root_vfs).owner_uid = root_inode.i_uid as u32;
    (*root_vfs).owner_gid = root_inode.i_gid as u32;
    (*root_vfs).rights = root_inode.i_mode & 0x0FFF;
    (*root_vfs).mount = mount_wrapper;
    if target.is_null() {
        vfs::ROOT_NODE = root_vfs;
    }
    let dir_block_num = root_inode.i_block[0];
    let dir_lba = dir_block_num * (mount.sb.block_size() / 512);
    let dir_alloc = kmalloc(4096)?;
    let dir_buffer = &mut *(dir_alloc as *mut [u8; 4096]);
    let read_res = drivers::read_sectors(
        mount.device_id,
        dir_lba,
        (mount.sb.block_size() / 512) as u8,
        dir_buffer,
    );
    if let Err(e) = read_res {
        kfree(dir_alloc).ok();
        return Err(e);
    }

    parse_directory_block(
        &mount,
        dir_buffer.as_ptr(),
        mount.sb.block_size() as usize,
        root_vfs,
        root_vfs,
    );
    kfree(dir_alloc).ok();

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
    let mount = ext2_mount_from_wrapper(mount_wrap)?;
    let inode = get_inode(&mount, inode_num)?;

    let file_size = inode.i_size;
    if offset >= file_size {
        return Ok(0); // EOF
    }

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

    let block_size = mount.sb.block_size() as usize;
    let space_in_block = block_size - offset_in_block;
    let bytes_to_read = core::cmp::min(
        core::cmp::min(buffer.len(), (file_size - offset) as usize),
        space_in_block,
    );

    let block_alloc = kmalloc(4096)?;
    let block_buf = &mut *(block_alloc as *mut [u8; 4096]);
    if read_block(&mount, physical_block, &mut block_buf[..block_size]).is_err() {
        kfree(block_alloc).ok();
        return Err(KernelError::EIO);
    }

    core::ptr::copy_nonoverlapping(
        block_buf.as_ptr().add(offset_in_block),
        buffer.as_mut_ptr(),
        bytes_to_read,
    );
    kfree(block_alloc).ok();

    Ok(bytes_to_read)
}

pub unsafe fn write_to_inode(
    mount_wrap: *const Mount,
    inode_num: u32,
    buffer: &[u8],
    mut offset: u32,
) -> KResult<usize> {
    let mount = ext2_mount_from_wrapper(mount_wrap)?;
    let mut bytes_written = 0;
    let mut remaining = buffer.len();

    while remaining > 0 {
        let logical_block = offset / mount.sb.block_size();
        let block_offset = offset % mount.sb.block_size();
        let space_in_block = mount.sb.block_size() - block_offset;
        let write_size = core::cmp::min(remaining, space_in_block as usize);
        let phys_block = get_or_allocate_physical_block(&mount, inode_num, logical_block)?;

        let block_alloc = kmalloc(4096)?;
        let block_buf = &mut *(block_alloc as *mut [u8; 4096]);

        let src_start = bytes_written;
        let src_end = src_start + write_size;
        let dst_start = block_offset as usize;
        let dst_end = dst_start + write_size;

        let result = (|| {
            if write_size < mount.sb.block_size() as usize {
                read_block(&mount, phys_block, block_buf)?;
            }
            block_buf[dst_start..dst_end].copy_from_slice(&buffer[src_start..src_end]);
            write_block(&mount, phys_block, block_buf)
        })();
        kfree(block_alloc).ok();
        result?;

        bytes_written += write_size;
        remaining -= write_size;
        offset += write_size as u32;
    }

    let mut inode = get_inode(&mount, inode_num)?;
    if offset > inode.i_size {
        inode.i_size = offset;
        update_inode(&mount, inode_num, &inode)?;
    }

    Ok(bytes_written)
}

pub unsafe fn truncate_inode(mount_wrap: *const Mount, inode_num: u32) -> KResult<()> {
    let mount = ext2_mount_from_wrapper(mount_wrap)?;
    let mut inode = get_inode(&mount, inode_num)?;

    for block_index in 0..12 {
        let block_num = inode.i_block[block_index];
        if block_num != 0 {
            ext2_free_block(&mount, block_num)?;
            inode.i_block[block_index] = 0;
        }
    }

    if inode.i_block[12] != 0 {
        free_block_tree(&mount, inode.i_block[12], 1)?;
        inode.i_block[12] = 0;
    }

    if inode.i_block[13] != 0 {
        free_block_tree(&mount, inode.i_block[13], 2)?;
        inode.i_block[13] = 0;
    }

    if inode.i_block[14] != 0 {
        free_block_tree(&mount, inode.i_block[14], 3)?;
        inode.i_block[14] = 0;
    }

    inode.i_size = 0;
    inode.i_blocks = 0;
    update_inode(&mount, inode_num, &inode)
}

pub unsafe fn lazy_load_directory(
    mount_wrap: *const Mount,
    dir_node: *mut VfsNode,
    dir_node_num: u32,
) -> KResult<()> {
    let mount = ext2_mount_from_wrapper(mount_wrap)?;
    let inode = get_inode(&mount, dir_node_num)?;
    // For simplicity, we only read the first direct block of the directory.
    // Large directories require reading i_block[1], i_block[2], etc.
    let block_num = inode.i_block[0];
    if block_num == 0 {
        return Ok(());
    }

    let lba = block_num * (mount.sb.block_size() / 512);
    let sector_count = (mount.sb.block_size() / 512) as u8;

    let dir_alloc = kmalloc(4096)?;
    let dir_buf = &mut *(dir_alloc as *mut [u8; 4096]);
    let read_res = drivers::read_sectors(mount.device_id, lba, sector_count, dir_buf);
    if let Err(e) = read_res {
        kfree(dir_alloc).ok();
        return Err(e);
    }

    parse_directory_block(
        &mount,
        dir_buf.as_ptr(),
        mount.sb.block_size() as usize,
        dir_node,
        (*dir_node).master,
    );
    kfree(dir_alloc).ok();
    Ok(())
}
