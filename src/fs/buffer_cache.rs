use crate::drivers::{self, SECTOR_SIZE};
use crate::error::{KResult, KernelError};

const CACHE_SIZE: usize = 64;
const BUFFER_SIZE: usize = 4096;

#[derive(Clone, Copy)]
pub struct BufferHead {
    pub device_id: u32,
    pub block_num: u32,
    pub data: [u8; BUFFER_SIZE],
    pub is_dirty: bool,
    pub is_valid: bool,
    pub ref_count: u32,
    pub last_accessed: u64,
}

impl BufferHead {
    const fn empty() -> Self {
        Self {
            device_id: 0,
            block_num: 0,
            data: [0; BUFFER_SIZE],
            is_dirty: false,
            is_valid: false,
            ref_count: 0,
            last_accessed: 0,
        }
    }
}

static mut BUFFER_CACHE: [BufferHead; CACHE_SIZE] = [BufferHead::empty(); CACHE_SIZE];
static mut ACCESS_COUNTER: u64 = 0;

unsafe fn flush_buffer(bh: &mut BufferHead) -> KResult<()> {
    if !bh.is_dirty || !bh.is_valid {
        return Ok(());
    }

    let device = drivers::device_at(bh.device_id as usize).ok_or(KernelError::ENODEV)?;
    let sectors_per_block = (BUFFER_SIZE / SECTOR_SIZE) as u8;
    let lba = bh.block_num * (sectors_per_block as u32);

    (device.write)(&device, lba, sectors_per_block, &bh.data)?;
    bh.is_dirty = false;
    Ok(())
}

pub unsafe fn bread(device_id: u32, block_num: u32) -> KResult<&'static mut BufferHead> {
    ACCESS_COUNTER += 1;

    for i in 0..CACHE_SIZE {
        let bh = &mut BUFFER_CACHE[i];
        if bh.is_valid && bh.device_id == device_id && bh.block_num == block_num {
            bh.ref_count += 1;
            bh.last_accessed = ACCESS_COUNTER;
            return Ok(bh);
        }
    }

    let mut eviction_idx = CACHE_SIZE;
    let mut oldest_access = u64::MAX;

    for i in 0..CACHE_SIZE {
        let bh = &BUFFER_CACHE[i];
        if bh.ref_count == 0 {
            if !bh.is_valid {
                eviction_idx = i;
                break;
            }
            if bh.last_accessed < oldest_access {
                oldest_access = bh.last_accessed;
                eviction_idx = i;
            }
        }
    }

    if eviction_idx == CACHE_SIZE {
        crate::pr_err!("Buffer Cache: Out of memory, all buffers are locked.\n");
        return Err(KernelError::ENOMEM);
    }

    let bh = &mut BUFFER_CACHE[eviction_idx];

    if bh.is_dirty {
        flush_buffer(bh)?;
    }

    let device = drivers::device_at(device_id as usize).ok_or(KernelError::ENODEV)?;
    let sectors_per_block = (BUFFER_SIZE / SECTOR_SIZE) as u8;
    let lba = block_num * (sectors_per_block as u32);

    (device.read)(&device, lba, sectors_per_block, &mut bh.data)?;

    bh.device_id = device_id;
    bh.block_num = block_num;
    bh.is_dirty = false;
    bh.is_valid = true;
    bh.ref_count = 1;
    bh.last_accessed = ACCESS_COUNTER;

    Ok(bh)
}

pub unsafe fn bwrite(bh: &mut BufferHead) {
    bh.is_dirty = true;
}

pub unsafe fn brelse(bh: &mut BufferHead) {
    if bh.ref_count > 0 {
        bh.ref_count -= 1;
    } else {
        crate::pr_warn!("Buffer Cache: Attempted to release an unreferenced buffer.\n");
    }
}

pub unsafe fn bsync() -> KResult<()> {
    for i in 0..CACHE_SIZE {
        let bh = &mut BUFFER_CACHE[i];
        if bh.is_dirty {
            flush_buffer(bh)?;
        }
    }
    Ok(())
}
