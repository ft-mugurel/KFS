use super::frame_allocator;
use super::kernel_heap;
use super::multiboot::{
    MULTIBOOT_BOOTLOADER_MAGIC, MULTIBOOT_INFO_HAS_BASIC_MEMORY, capture_boot_cmdline,
    multiboot_info_from_addr, set_boot_multiboot_info_addr,
};
use super::page_table;
use super::vmem;

use crate::{pr_debug, x86};

#[unsafe(link_section = ".init.text")]
pub fn init_paging(multiboot_magic: u32, multiboot_info_addr: u32) {
    assert_eq!(
        multiboot_magic, MULTIBOOT_BOOTLOADER_MAGIC,
        "Invalid multiboot magic: {:#x}",
        multiboot_magic
    );

    let mb_info = multiboot_info_from_addr(multiboot_info_addr);
    set_boot_multiboot_info_addr(multiboot_info_addr);
    capture_boot_cmdline(mb_info);

    if (mb_info.flags & MULTIBOOT_INFO_HAS_BASIC_MEMORY) != 0 {
        pr_debug!(
            "Multiboot low/high memory: {} KiB / {} KiB",
            mb_info.mem_lower,
            mb_info.mem_upper
        );
    }

    frame_allocator::init_from_multiboot(mb_info);
    vmem::init_vmem();
    unsafe { kernel_heap::init_kernel_heap() };

    pr_debug!(
        "Physical frames: total={} free={}",
        frame_allocator::total_frame_count(),
        frame_allocator::free_frame_count()
    );

    unsafe { page_table::enable_bootstrap_paging() };
    pr_debug!(
        "Paging enabled: cr3={:#x} bootstrap_pd={:#x}",
        x86::read_cr3(),
        page_table::bootstrap_directory_phys_addr()
    );

    page_table::mark_paging_initialized();
}
