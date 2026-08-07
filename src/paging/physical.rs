use crate::error::KResult;

use super::{frame_allocator, PAGE_SIZE};

pub fn alloc_physical_page() -> KResult<u32> {
    frame_allocator::alloc_frame()
}

pub fn alloc_physical_page_below(limit_addr: u64) -> KResult<u32> {
    frame_allocator::alloc_frame_below(limit_addr)
}

pub fn free_physical_page(phys_addr: u32) -> KResult<()> {
    frame_allocator::free_frame(phys_addr)
}

pub fn total_physical_pages() -> usize {
    frame_allocator::total_frame_count()
}

pub fn free_physical_pages() -> usize {
    frame_allocator::free_frame_count()
}

pub const fn physical_page_size() -> usize {
    PAGE_SIZE
}
