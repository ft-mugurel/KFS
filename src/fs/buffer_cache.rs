use crate::{
    drivers::{self, SECTOR_SIZE},
    error::{KResult, KernelError},
    locks::Spinlock,
};

pub const CACHE_SIZE: usize = 64;
pub const BUFFER_SIZE: usize = 4096;

#[derive(Clone, Copy)]
pub struct BufferHead {
    pub device_id: u32,
    pub block_num: u32,
    pub block_size: u32,
    pub data: [u8; BUFFER_SIZE],
    pub is_dirty: bool,
    pub is_valid: bool,
    pub io_in_progress: bool,
    pub ref_count: u32,
    pub last_accessed: u64,
}

impl BufferHead {
    const fn empty() -> Self {
        Self {
            device_id: 0,
            block_num: 0,
            block_size: BUFFER_SIZE as u32,
            data: [0; BUFFER_SIZE],
            is_dirty: false,
            is_valid: false,
            io_in_progress: false,
            ref_count: 0,
            last_accessed: 0,
        }
    }
}

struct BufferCacheState {
    buffers: [BufferHead; CACHE_SIZE],
    access_counter: u64,
}

impl BufferCacheState {
    const fn new() -> Self {
        Self {
            buffers: [BufferHead::empty(); CACHE_SIZE],
            access_counter: 0,
        }
    }
}

static BUFFER_CACHE: Spinlock<BufferCacheState> = Spinlock::new(BufferCacheState::new());

const MAX_WAIT_CYCLES: usize = 1_000_000;

pub unsafe fn bread(device_id: u32, block_num: u32, block_size: u32) -> KResult<*mut BufferHead> {
    if block_size == 0 || (block_size as usize) > BUFFER_SIZE {
        return Err(KernelError::EINVAL);
    }

    let mut attempts = 0;

    loop {
        let mut state = BUFFER_CACHE.lock();
        state.access_counter = state.access_counter.wrapping_add(1);
        let counter = state.access_counter;

        // 1. Check if the block is already in the cache
        let mut found_idx = None;
        let mut in_progress = false;

        for i in 0..CACHE_SIZE {
            let bh = &state.buffers[i];
            if bh.device_id == device_id
                && bh.block_num == block_num
                && (bh.is_valid || bh.io_in_progress)
            {
                if bh.io_in_progress {
                    in_progress = true;
                } else if bh.is_valid {
                    found_idx = Some(i);
                }
                break;
            }
        }

        if in_progress {
            drop(state);
            core::hint::spin_loop();
            attempts += 1;
            if attempts > MAX_WAIT_CYCLES {
                crate::pr_err!(
                    "Buffer Cache: Timeout waiting for block {} on device {}\n",
                    block_num,
                    device_id
                );
                return Err(KernelError::EBUSY);
            }
            continue;
        }

        if let Some(idx) = found_idx {
            let bh = &mut state.buffers[idx];
            bh.ref_count += 1;
            bh.last_accessed = counter;
            let ptr = bh as *mut BufferHead;
            drop(state);
            return Ok(ptr);
        }

        // 2. Not in cache: find an unreferenced victim
        let mut eviction_idx = None;
        let mut oldest_access = u64::MAX;

        for i in 0..CACHE_SIZE {
            let bh = &state.buffers[i];
            if bh.ref_count == 0 && !bh.io_in_progress {
                if !bh.is_valid {
                    eviction_idx = Some(i);
                    break;
                }
                if bh.last_accessed < oldest_access {
                    oldest_access = bh.last_accessed;
                    eviction_idx = Some(i);
                }
            }
        }

        let Some(idx) = eviction_idx else {
            drop(state);
            core::hint::spin_loop();
            attempts += 1;
            if attempts > MAX_WAIT_CYCLES {
                crate::pr_err!(
                    "Buffer Cache: All {} buffers locked, deadlock avoided\n",
                    CACHE_SIZE
                );
                return Err(KernelError::ENOMEM);
            }
            continue;
        };

        let bh = &mut state.buffers[idx];

        // 3. If victim is dirty, flush it without holding BUFFER_CACHE lock
        if bh.is_dirty && bh.is_valid {
            bh.io_in_progress = true;
            bh.ref_count = 1;
            let old_dev = bh.device_id;
            let old_block = bh.block_num;
            let old_size = bh.block_size;
            let sectors_per_block = (old_size / SECTOR_SIZE as u32) as u8;
            let lba = old_block * (sectors_per_block as u32);
            let mut flush_data = [0u8; BUFFER_SIZE];
            flush_data[..old_size as usize].copy_from_slice(&bh.data[..old_size as usize]);

            drop(state);

            let res = drivers::write_sectors(
                old_dev as usize,
                lba,
                sectors_per_block,
                &flush_data[..old_size as usize],
            );

            let mut state = BUFFER_CACHE.lock();
            let bh = &mut state.buffers[idx];
            bh.io_in_progress = false;
            bh.ref_count = 0;
            if let Err(e) = res {
                crate::pr_err!("Buffer Cache: Failed to flush victim block {}\n", old_block);
                return Err(e);
            }
            bh.is_dirty = false;
            bh.is_valid = false;
            // Retry allocation from clean state
            continue;
        }

        // 4. Reserve victim buffer for loading
        bh.device_id = device_id;
        bh.block_num = block_num;
        bh.block_size = block_size;
        bh.is_dirty = false;
        bh.is_valid = false;
        bh.io_in_progress = true;
        bh.ref_count = 1;
        bh.last_accessed = counter;

        drop(state);

        // 5. Read from disk without holding BUFFER_CACHE lock
        let sectors_per_block = (block_size / SECTOR_SIZE as u32) as u8;
        let lba = block_num * (sectors_per_block as u32);
        let mut read_data = [0u8; BUFFER_SIZE];
        let res = drivers::read_sectors(
            device_id as usize,
            lba,
            sectors_per_block,
            &mut read_data[..block_size as usize],
        );

        let mut state = BUFFER_CACHE.lock();
        let bh = &mut state.buffers[idx];
        bh.io_in_progress = false;

        if let Err(e) = res {
            bh.is_valid = false;
            bh.ref_count = 0;
            return Err(e);
        }

        bh.data[..block_size as usize].copy_from_slice(&read_data[..block_size as usize]);
        bh.is_valid = true;
        let ptr = bh as *mut BufferHead;
        drop(state);

        return Ok(ptr);
    }
}

pub unsafe fn bwrite(bh: *mut BufferHead) {
    if bh.is_null() {
        return;
    }
    let _guard = BUFFER_CACHE.lock();
    (*bh).is_dirty = true;
}

pub unsafe fn brelse(bh: *mut BufferHead) {
    if bh.is_null() {
        return;
    }
    let _guard = BUFFER_CACHE.lock();
    if (*bh).ref_count > 0 {
        (*bh).ref_count -= 1;
    } else {
        crate::pr_warn!("Buffer Cache: Attempted to release unreferenced buffer.\n");
    }
}

pub unsafe fn bsync() -> KResult<()> {
    for i in 0..CACHE_SIZE {
        let mut state = BUFFER_CACHE.lock();
        let bh = &mut state.buffers[i];

        if !bh.is_dirty || !bh.is_valid || bh.io_in_progress {
            continue;
        }

        bh.io_in_progress = true;
        bh.ref_count += 1;

        let dev = bh.device_id;
        let block = bh.block_num;
        let size = bh.block_size;
        let sectors_per_block = (size / SECTOR_SIZE as u32) as u8;
        let lba = block * (sectors_per_block as u32);
        let mut flush_data = [0u8; BUFFER_SIZE];
        flush_data[..size as usize].copy_from_slice(&bh.data[..size as usize]);

        drop(state);

        let res = drivers::write_sectors(
            dev as usize,
            lba,
            sectors_per_block,
            &flush_data[..size as usize],
        );

        let mut state = BUFFER_CACHE.lock();
        let bh = &mut state.buffers[i];
        bh.io_in_progress = false;
        if bh.ref_count > 0 {
            bh.ref_count -= 1;
        }
        if res.is_ok() {
            bh.is_dirty = false;
        } else {
            return res;
        }
    }

    Ok(())
}

pub unsafe fn bforget(device_id: u32, block_num: u32) {
    let mut state = BUFFER_CACHE.lock();
    for i in 0..CACHE_SIZE {
        let bh = &mut state.buffers[i];
        if bh.is_valid && bh.device_id == device_id && bh.block_num == block_num {
            bh.is_valid = false;
            bh.is_dirty = false;
            bh.ref_count = 0;
            break;
        }
    }
}

pub unsafe fn read_block_cached(
    device_id: u32,
    block_num: u32,
    block_size: u32,
    buffer: &mut [u8],
) -> KResult<()> {
    let bh = bread(device_id, block_num, block_size)?;
    let to_copy = (block_size as usize).min(buffer.len());
    let bh_ref = &*bh;
    buffer[..to_copy].copy_from_slice(&bh_ref.data[..to_copy]);
    brelse(bh);
    Ok(())
}

pub unsafe fn write_block_cached(
    device_id: u32,
    block_num: u32,
    block_size: u32,
    buffer: &[u8],
) -> KResult<()> {
    let bh = bread(device_id, block_num, block_size)?;
    let to_copy = (block_size as usize).min(buffer.len());
    let bh_ref = &mut *bh;
    bh_ref.data[..to_copy].copy_from_slice(&buffer[..to_copy]);
    bwrite(bh);
    brelse(bh);
    Ok(())
}
