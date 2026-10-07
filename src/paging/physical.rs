use crate::error::KResult;

use super::{frame_allocator, PAGE_SIZE};

pub fn alloc_physical_page() -> KResult<u32> {
    frame_allocator::alloc_frame()
}

pub fn alloc_physical_page_below(limit_addr: u64) -> KResult<u32> {
    frame_allocator::alloc_frame_below(limit_addr)
}

pub fn alloc_contiguous_physical_pages_below(
    count: usize,
    align_frames: usize,
    limit_addr: u64,
) -> KResult<u32> {
    frame_allocator::alloc_contiguous_frames_below(count, align_frames, limit_addr)
}

pub fn free_physical_page(phys_addr: u32) -> KResult<()> {
    frame_allocator::free_frame(phys_addr)
}

pub fn free_contiguous_physical_pages(phys_addr: u32, count: usize) -> KResult<()> {
    frame_allocator::free_contiguous_frames(phys_addr, count)
}

pub fn total_physical_pages() -> usize {
    frame_allocator::total_frame_count()
}

pub fn free_physical_pages() -> usize {
    frame_allocator::free_frame_count()
}

pub fn frame_ref_inc(phys_addr: u32) {
    frame_allocator::frame_ref_inc(phys_addr)
}

pub fn frame_ref_dec(phys_addr: u32) -> u8 {
    frame_allocator::frame_ref_dec(phys_addr)
}

pub fn frame_ref_count(phys_addr: u32) -> u8 {
    frame_allocator::frame_ref_count(phys_addr)
}

pub const fn physical_page_size() -> usize {
    PAGE_SIZE
}
