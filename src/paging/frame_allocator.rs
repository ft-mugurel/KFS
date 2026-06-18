use super::init::PAGE_SIZE;
use super::multiboot::{
    MemoryMapIter, MultibootInfo, MULTIBOOT_INFO_HAS_BASIC_MEMORY, MULTIBOOT_MEMORY_AVAILABLE,
};
use crate::spin::Spinlock;
use crate::{pr_debug, pr_warn};

const MAX_PHYS_MEM_BYTES: u64 = 4 * 1024 * 1024 * 1024;
// On 32-bit targets, casting 4 GiB to usize wraps to 0. Compute max 4 KiB frames
// from the highest 32-bit physical address instead.
const MAX_FRAMES: usize = (u32::MAX as usize / PAGE_SIZE) + 1;
const BITMAP_WORD_BITS: usize = 32;
const BITMAP_WORDS: usize = MAX_FRAMES / BITMAP_WORD_BITS;

unsafe extern "C" {
    static __kernel_end: u8;
}

#[inline]
const fn frame_index(phys_addr: u32) -> usize {
    (phys_addr as usize) / PAGE_SIZE
}

#[inline]
const fn frame_addr(frame_idx: usize) -> u32 {
    (frame_idx * PAGE_SIZE) as u32
}

#[inline]
const fn align_up(value: usize, align: usize) -> usize {
    (value + (align - 1)) & !(align - 1)
}

struct PhysicalState {
    bitmap: [u32; BITMAP_WORDS],
    total_frames: usize,
    free_frames: usize,
}

impl PhysicalState {
    const fn new() -> Self {
        Self {
            bitmap: [u32::MAX; BITMAP_WORDS],
            total_frames: MAX_FRAMES,
            free_frames: 0,
        }
    }

    #[inline]
    fn mark_used(&mut self, frame_idx: usize) {
        let word = frame_idx / BITMAP_WORD_BITS;
        let bit = frame_idx % BITMAP_WORD_BITS;
        let mask = 1u32 << bit;

        let was_free = (self.bitmap[word] & mask) == 0;
        self.bitmap[word] |= mask;
        if was_free {
            self.free_frames = self.free_frames.saturating_sub(1);
        }
    }

    #[inline]
    fn mark_free(&mut self, frame_idx: usize) {
        let word = frame_idx / BITMAP_WORD_BITS;
        let bit = frame_idx % BITMAP_WORD_BITS;
        let mask = 1u32 << bit;

        let was_used = (self.bitmap[word] & mask) != 0;
        self.bitmap[word] &= !mask;
        if was_used {
            self.free_frames = self.free_frames.saturating_add(1);
        }
    }

    #[inline]
    fn mark_range_used(&mut self, start_addr: u64, end_addr_exclusive: u64) {
        let start = (start_addr as usize / PAGE_SIZE).min(MAX_FRAMES);
        let end = align_up(end_addr_exclusive as usize, PAGE_SIZE)
            .saturating_div(PAGE_SIZE)
            .min(MAX_FRAMES);

        for frame_idx in start..end {
            self.mark_used(frame_idx);
        }
    }

    #[inline]
    fn mark_range_free(&mut self, start_addr: u64, end_addr_exclusive: u64) {
        let start = align_up(start_addr as usize, PAGE_SIZE)
            .saturating_div(PAGE_SIZE)
            .min(MAX_FRAMES);
        let end = (end_addr_exclusive as usize / PAGE_SIZE).min(MAX_FRAMES);

        for frame_idx in start..end {
            self.mark_free(frame_idx);
        }
    }
}

static ALLOCATOR_STATE: Spinlock<PhysicalState> = Spinlock::new(PhysicalState::new());

pub(super) fn init_from_multiboot(info: &MultibootInfo) {
    let mut state = ALLOCATOR_STATE.lock();
    // Start with every frame reserved, then free only bootloader-reported usable ranges.
    for i in 0usize..BITMAP_WORDS {
        state.bitmap[i] = u32::MAX;
    }
    state.total_frames = MAX_FRAMES;
    state.free_frames = 0;

    let mut has_usable_memory = false;

    if let Some(iter) = MemoryMapIter::new(info) {
        for region in iter {
            if region.region_type == MULTIBOOT_MEMORY_AVAILABLE {
                let start = region.base_addr.min(MAX_PHYS_MEM_BYTES);
                let end = region
                    .base_addr
                    .saturating_add(region.length)
                    .min(MAX_PHYS_MEM_BYTES);
                if end > start {
                    let free_before = state.free_frames;
                    state.mark_range_free(start, end);
                    if state.free_frames > free_before {
                        has_usable_memory = true;
                    }
                }
            }
        }
    }

    // Some multiboot paths only provide mem_lower/mem_upper, and some mmap tables are
    // present but unusable in practice. If mmap freed nothing, fall back to mem_upper.
    if (!has_usable_memory || state.free_frames == 0)
        && (info.flags & MULTIBOOT_INFO_HAS_BASIC_MEMORY) != 0
    {
        let fallback_start = 0x0010_0000u64;
        let fallback_end = fallback_start
            .saturating_add((info.mem_upper as u64).saturating_mul(1024))
            .min(MAX_PHYS_MEM_BYTES);

        if fallback_end > fallback_start {
            let free_before = state.free_frames;
            state.mark_range_free(fallback_start, fallback_end);
            if state.free_frames > free_before {
                has_usable_memory = true;
            }
            pr_warn!(
                "mmap yielded no free pages; using mem_upper fallback range [{:#x}, {:#x})\n",
                fallback_start as usize,
                fallback_end as usize
            );
        }
    }

    // Keep low memory reserved and keep the loaded kernel image reserved.
    state.mark_range_used(0, 0x0010_0000);
    let kernel_end = align_up(&raw const __kernel_end as usize, PAGE_SIZE) as u64;
    state.mark_range_used(0x0010_0000, kernel_end);
    let total_frames = state.total_frames;
    let free_frames = state.free_frames;

    pr_debug!(
            "Frame allocator initialized: max_frames={} free_frames={} reserved_low=0x{:x} kernel_end=0x{:x}\n",
            total_frames,
            free_frames,
            0x0010_0000usize,
            kernel_end as usize
        );

    if !has_usable_memory || state.free_frames == 0 {
        pr_warn!(
            "no usable physical frames discovered (flags={:#x}, mem_upper={} KiB)\n",
            info.flags,
            info.mem_upper
        );
    }
}

#[inline(never)]
pub(super) fn alloc_frame() -> Option<u32> {
    let mut state = ALLOCATOR_STATE.lock();
    for word_idx in 0..BITMAP_WORDS {
        let word = state.bitmap[word_idx];
        if word != u32::MAX {
            let bit = (!word).trailing_zeros() as usize;
            let frame_idx = word_idx * BITMAP_WORD_BITS + bit;
            if frame_idx < MAX_FRAMES {
                state.mark_used(frame_idx);
                let addr = frame_addr(frame_idx);
                let free_frames = state.free_frames;
                pr_debug!("alloc_frame -> {:#x} (free_left={})\n", addr, free_frames);
                return Some(addr);
            }
        }
    }

    pr_warn!("alloc_frame failed: no free physical frame\n");
    None
}

#[inline(never)]
pub(super) fn alloc_frame_below(limit_addr: u64) -> Option<u32> {
    let mut state = ALLOCATOR_STATE.lock();
    let limit_frame = (limit_addr as usize / PAGE_SIZE).min(MAX_FRAMES);
    let mut frame_idx = limit_frame;
    for idx in 0usize..limit_frame {
        let word = idx / BITMAP_WORD_BITS;
        let bit = idx % BITMAP_WORD_BITS;
        if (state.bitmap[word] & (1u32 << bit)) == 0 {
            frame_idx = idx;
            break;
        }
    }

    if frame_idx >= limit_frame {
        pr_warn!(
            "alloc_frame_below({:#x}) failed: no free frame below limit\n",
            limit_addr
        );
        return None;
    }

    state.mark_used(frame_idx);
    let addr = frame_addr(frame_idx);
    let free_frames = state.free_frames;
    pr_debug!(
        "alloc_frame_below({:#x}) -> {:#x} (free_left={})\n",
        limit_addr,
        addr,
        free_frames
    );
    Some(addr)
}

#[inline(never)]
pub(super) fn free_frame(phys_addr: u32) -> bool {
    let frame_idx = frame_index(phys_addr);
    if frame_idx >= MAX_FRAMES || (phys_addr as usize % PAGE_SIZE) != 0 {
        pr_warn!("free_frame rejected invalid addr={:#x}\n", phys_addr);
        return false;
    }

    let mut state = ALLOCATOR_STATE.lock();
    state.mark_free(frame_idx);
    let free_frames = state.free_frames;
    pr_debug!(
        "free_frame <- {:#x} (free_now={})\n",
        phys_addr,
        free_frames
    );
    true
}

pub(super) fn total_frame_count() -> usize {
    ALLOCATOR_STATE.lock().total_frames
}

pub(super) fn free_frame_count() -> usize {
    ALLOCATOR_STATE.lock().free_frames
}
