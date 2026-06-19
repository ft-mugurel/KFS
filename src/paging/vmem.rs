use super::init::{KERNEL_SPACE_START, PAGE_SIZE};
use super::page_table;
use super::physical;
use crate::{pr_debug, pr_warn};

const VMALLOC_START: u32 = KERNEL_SPACE_START as u32 + 0x0040_0000;
const VMALLOC_END: u32 = KERNEL_SPACE_START as u32 + 0x0100_0000;
const VMALLOC_SIZE: usize = (VMALLOC_END - VMALLOC_START) as usize;
const MAX_VIRTUAL_ALLOCS: usize = 128;
const MAX_FREE_RANGES: usize = 128;
const MAX_PAGES_PER_ALLOC: usize = VMALLOC_SIZE / PAGE_SIZE;

#[derive(Clone, Copy)]
struct VirtualAllocRecord {
    in_use: bool,
    base: u32,
    size: usize,
    page_count: usize,
    frames: [u32; MAX_PAGES_PER_ALLOC],
}

#[derive(Clone, Copy)]
struct FreeRange {
    base: u32,
    size: u32,
}

const EMPTY_ALLOC_RECORD: VirtualAllocRecord = VirtualAllocRecord {
    in_use: false,
    base: 0,
    size: 0,
    page_count: 0,
    frames: [0; MAX_PAGES_PER_ALLOC],
};

static mut VIRTUAL_ALLOCS: [VirtualAllocRecord; MAX_VIRTUAL_ALLOCS] =
    [EMPTY_ALLOC_RECORD; MAX_VIRTUAL_ALLOCS];
static mut FREE_RANGES: [FreeRange; MAX_FREE_RANGES] =
    [FreeRange { base: 0, size: 0 }; MAX_FREE_RANGES];
static mut FREE_RANGE_COUNT: usize = 0;

#[derive(Clone, Copy)]
pub struct VmemStats {
    pub range_start: u32,
    pub range_end: u32,
    pub total_bytes: u32,
    pub free_bytes: u32,
    pub free_ranges: usize,
    pub alloc_count: usize,
    pub alloc_bytes: usize,
}

fn page_count_for(size: usize) -> usize {
    (size + PAGE_SIZE - 1) / PAGE_SIZE
}

fn pages_bytes(pages: usize) -> Option<u32> {
    let bytes = pages.checked_mul(PAGE_SIZE)?;
    if bytes > u32::MAX as usize {
        None
    } else {
        Some(bytes as u32)
    }
}

pub fn init_vmem() {
    unsafe {
        for i in 0usize..MAX_VIRTUAL_ALLOCS {
            VIRTUAL_ALLOCS[i] = EMPTY_ALLOC_RECORD;
        }

        for i in 0usize..FREE_RANGE_COUNT {
            FREE_RANGES[i] = FreeRange { base: 0, size: 0 };
        }

        FREE_RANGES[0] = FreeRange { base: VMALLOC_START, size: VMALLOC_END - VMALLOC_START };
        FREE_RANGE_COUNT = 1;

        pr_debug!(
            "vmem initialized: range=[{:#x}, {:#x}) free_ranges=1\n",
            VMALLOC_START,
            VMALLOC_END
        );
    }
}

fn find_free_slot() -> Option<usize> {
    for i in 0usize..MAX_VIRTUAL_ALLOCS {
        unsafe {
            if !VIRTUAL_ALLOCS[i].in_use {
                return Some(i);
            }
        }
    }
    None
}

fn find_slot_by_base(base: u32) -> Option<usize> {
    for i in 0usize..MAX_VIRTUAL_ALLOCS {
        unsafe {
            let rec = VIRTUAL_ALLOCS[i];
            if rec.in_use && rec.base == base {
                return Some(i);
            }
        }
    }
    None
}

unsafe fn rollback_alloc(base: u32, mapped_pages: usize, frames: &[u32; MAX_PAGES_PER_ALLOC]) {
    for i in 0usize..mapped_pages {
        let va = base + (i * PAGE_SIZE) as u32;
        let _ = page_table::unmap_page(va);
        let _ = physical::free_physical_page(frames[i]);
    }
}

fn allocate_virtual_span(span: u32) -> Option<u32> {
    unsafe {
        for i in 0usize..FREE_RANGE_COUNT {
            let range = FREE_RANGES[i];
            if range.size >= span {
                let base = range.base;
                let remaining = range.size - span;
                if remaining == 0 {
                    for j in i..(FREE_RANGE_COUNT - 1) {
                        FREE_RANGES[j] = FREE_RANGES[j + 1];
                    }
                    FREE_RANGE_COUNT -= 1;
                } else {
                    FREE_RANGES[i] = FreeRange { base: range.base + span, size: remaining };
                }
                return Some(base);
            }
        }
    }

    None
}

fn insert_free_range(mut base: u32, mut size: u32) -> bool {
    if size == 0 {
        return true;
    }

    let end = match base.checked_add(size) {
        Some(v) => v,
        None => return false,
    };

    let mut i = 0usize;
    unsafe {
        while i < FREE_RANGE_COUNT {
            let range = FREE_RANGES[i];
            let range_end = match range.base.checked_add(range.size) {
                Some(v) => v,
                None => return false,
            };

            if end == range.base {
                base = range.base;
                size = size.saturating_add(range.size);
                let mut j = i;
                while j + 1 < FREE_RANGE_COUNT {
                    FREE_RANGES[j] = FREE_RANGES[j + 1];
                    j += 1;
                }
                FREE_RANGE_COUNT -= 1;
                i = 0;
                continue;
            }

            if range_end == base {
                base = range.base;
                size = size.saturating_add(range.size);
                let mut j = i;
                while j + 1 < FREE_RANGE_COUNT {
                    FREE_RANGES[j] = FREE_RANGES[j + 1];
                    j += 1;
                }
                FREE_RANGE_COUNT -= 1;
                i = 0;
                continue;
            }

            i += 1;
        }

        if FREE_RANGE_COUNT >= MAX_FREE_RANGES {
            return false;
        }

        let mut insert_at = 0usize;
        while insert_at < FREE_RANGE_COUNT && FREE_RANGES[insert_at].base < base {
            insert_at += 1;
        }

        let mut j = FREE_RANGE_COUNT;
        while j > insert_at {
            FREE_RANGES[j] = FREE_RANGES[j - 1];
            j -= 1;
        }

        FREE_RANGES[insert_at] = FreeRange { base, size };
        FREE_RANGE_COUNT += 1;
    }

    true
}

#[inline(never)]
pub fn vmalloc(size: usize) -> Option<*mut u8> {
    if size == 0 {
        pr_warn!("vmalloc rejected zero-sized request\n");
        return None;
    }

    let page_count = page_count_for(size);
    if page_count == 0 || page_count > MAX_PAGES_PER_ALLOC {
        pr_warn!(
            "vmalloc request too large size={} pages={} max_pages_per_alloc={}\n",
            size,
            page_count,
            MAX_PAGES_PER_ALLOC
        );
        return None;
    }

    unsafe {
        let span = match pages_bytes(page_count) {
            Some(v) => v,
            None => {
                pr_warn!("vmalloc size overflow size={} pages={}\n", size, page_count);
                return None;
            }
        };

        let base = match allocate_virtual_span(span) {
            Some(v) => v,
            None => {
                pr_warn!(
                    "vmalloc out of reusable virtual space size={} span={:#x}\n",
                    size,
                    span
                );
                return None;
            }
        };

        let end = match base.checked_add(span) {
            Some(v) => v,
            None => {
                pr_warn!(
                    "vmalloc virtual range overflow base={:#x} span={:#x}\n",
                    base,
                    span
                );
                let _ = insert_free_range(base, span);
                return None;
            }
        };

        if end > VMALLOC_END {
            pr_warn!(
                "vmalloc out of virtual space base={:#x} end={:#x} limit={:#x}\n",
                base,
                end,
                VMALLOC_END
            );
            let _ = insert_free_range(base, span);
            return None;
        }

        let slot = match find_free_slot() {
            Some(v) => v,
            None => {
                pr_warn!("vmalloc allocation table full\n");
                return None;
            }
        };

        let mut frames = [0u32; MAX_PAGES_PER_ALLOC];
        for mapped_pages in 0usize..page_count {
            let frame = match physical::alloc_physical_page() {
                Some(v) => v,
                None => {
                    pr_warn!(
                        "vmalloc ran out of physical pages at mapped_pages={}\n",
                        mapped_pages
                    );
                    rollback_alloc(base, mapped_pages, &frames);
                    return None;
                }
            };

            let va = base + (mapped_pages * PAGE_SIZE) as u32;
            if page_table::map_page(va, frame, page_table::PAGE_WRITABLE).is_err() {
                let _ = physical::free_physical_page(frame);
                rollback_alloc(base, mapped_pages, &frames);
                return None;
            }

            frames[mapped_pages] = frame;
        }

        VIRTUAL_ALLOCS[slot] = VirtualAllocRecord { in_use: true, base, size, page_count, frames };
        pr_debug!(
            "vmalloc size={} pages={} base={:#x} end={:#x}\n",
            size,
            page_count,
            base,
            end
        );
        Some(base as *mut u8)
    }
}

#[inline(never)]
pub fn vfree(ptr: *mut u8) -> bool {
    let base = ptr as u32;
    if base == 0 {
        pr_warn!("vfree rejected null pointer\n");
        return false;
    }

    unsafe {
        let slot = match find_slot_by_base(base) {
            Some(v) => v,
            None => {
                pr_warn!("vfree unknown pointer={:#x}\n", base);
                return false;
            }
        };

        let rec = VIRTUAL_ALLOCS[slot];
        for i in 0usize..rec.page_count {
            let va = rec.base + (i * PAGE_SIZE) as u32;
            let _ = page_table::unmap_page(va);
            let _ = physical::free_physical_page(rec.frames[i]);
        }

        VIRTUAL_ALLOCS[slot] = EMPTY_ALLOC_RECORD;
        let inserted = insert_free_range(base, (rec.page_count * PAGE_SIZE) as u32);
        if !inserted {
            pr_warn!(
                "vfree could not return virtual range to free list ptr={:#x}\n",
                base
            );
        }
        pr_debug!(
            "vfree ptr={:#x} size={} pages={}\n",
            base,
            rec.size,
            rec.page_count
        );
        true
    }
}

#[inline(never)]
pub fn vsize(ptr: *const u8) -> Option<usize> {
    let base = ptr as u32;
    if base == 0 {
        return None;
    }

    unsafe {
        for i in 0usize..MAX_VIRTUAL_ALLOCS {
            let rec = VIRTUAL_ALLOCS[i];
            if rec.in_use && rec.base == base {
                return Some(rec.size);
            }
        }
    }

    None
}

pub fn debug_stats() -> VmemStats {
    unsafe {
        let mut free_bytes = 0u32;
        for i in 0usize..FREE_RANGE_COUNT {
            free_bytes = free_bytes.saturating_add(FREE_RANGES[i].size);
        }

        let mut alloc_count = 0usize;
        let mut alloc_bytes = 0usize;
        for i in 0usize..MAX_VIRTUAL_ALLOCS {
            let rec = VIRTUAL_ALLOCS[i];
            if rec.in_use {
                alloc_count += 1;
                alloc_bytes = alloc_bytes.saturating_add(rec.size);
            }
        }

        VmemStats {
            range_start: VMALLOC_START,
            range_end: VMALLOC_END,
            total_bytes: VMALLOC_END - VMALLOC_START,
            free_bytes,
            free_ranges: FREE_RANGE_COUNT,
            alloc_count,
            alloc_bytes,
        }
    }
}

pub fn debug_for_each_alloc(mut f: impl FnMut(u32, usize, usize)) {
    unsafe {
        for i in 0usize..MAX_VIRTUAL_ALLOCS {
            let rec = VIRTUAL_ALLOCS[i];
            if rec.in_use {
                f(rec.base, rec.size, rec.page_count);
            }
        }
    }
}

pub fn debug_for_each_free_range(mut f: impl FnMut(u32, u32)) {
    unsafe {
        for i in 0usize..FREE_RANGE_COUNT {
            let range = FREE_RANGES[i];
            f(range.base, range.size);
        }
    }
}
