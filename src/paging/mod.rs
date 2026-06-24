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
pub(crate) const PAGE_PAGE_SIZE_4MB: u32 = 1 << 7;

pub(crate) use init::init_paging;
pub(crate) use kernel_heap::{debug_stats as kernel_heap_debug_stats, kfree, kmalloc, ksize};
pub(crate) use page_table::{
    bootstrap_directory_phys_addr, clone_address_space, free_user_address_space, get_page,
    get_physical_address, map_page, map_zero_page, phys_to_virt, unmap_page,
};
pub(crate) use physical::{
    alloc_physical_page, free_physical_page, free_physical_pages, physical_page_size,
    total_physical_pages,
};
pub(crate) use vmem::{
    debug_for_each_alloc as vmem_debug_for_each_alloc,
    debug_for_each_free_range as vmem_debug_for_each_free_range, debug_stats as vmem_debug_stats,
    vfree, vmalloc, vsize,
};
