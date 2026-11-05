mod early;
mod frame_allocator;
mod init;
mod kernel_heap;
mod multiboot;
mod page_table;
mod physical;
mod vmem;

pub(crate) const PAGE_PRESENT: u32 = 1 << 0;
pub(crate) const PAGE_WRITABLE: u32 = 1 << 1;
pub(crate) const PAGE_USER: u32 = 1 << 2;
pub(crate) const PAGE_PCD: u32 = 1 << 4; // cache-disable, required for MMIO (LAPIC/IOAPIC regs must not be cached)
pub(crate) const PAGE_PAGE_SIZE_4MB: u32 = 1 << 7;

pub(crate) const PAGE_SIZE: usize = 4096;
pub(crate) const USER_SPACE_START: usize = 0x0000_1000;
pub(crate) const KERNEL_SPACE_START: usize = 0xC000_0000;

pub(crate) use init::init_paging;
pub(crate) use kernel_heap::{
    HeapBuffer, debug_stats as kernel_heap_debug_stats, kfree, kmalloc, ksize,
    span_coalesce_selftest as kernel_heap_span_selftest,
};
pub(crate) use multiboot::boot_cmdline;
pub(crate) use page_table::{
    PAGE_TABLE_ALLOC_LIMIT, bootstrap_directory_phys_addr, clone_address_space,
    free_user_address_space, get_page, map_page, map_zero_page, page_entry_in, phys_to_virt,
    unmap_page, virt_to_phys,
};
pub(crate) use physical::{
    alloc_contiguous_physical_pages_below, alloc_physical_page, alloc_physical_page_below,
    frame_ref_count, frame_ref_dec, frame_ref_inc, free_contiguous_physical_pages,
    free_physical_page, free_physical_pages, physical_page_size, total_physical_pages,
};
pub(crate) use vmem::{
    debug_for_each_alloc as vmem_debug_for_each_alloc,
    debug_for_each_free_range as vmem_debug_for_each_free_range, debug_stats as vmem_debug_stats,
    ioremap, iounmap, vfree, vmalloc, vsize,
};

pub use early::read_phys;
