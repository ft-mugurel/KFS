use super::init::{KERNEL_SPACE_START, USER_SPACE_START};
use super::physical;
use crate::{pr_debug, pr_warn, x86};

pub const PAGE_PRESENT: u32 = 1 << 0;
pub const PAGE_WRITABLE: u32 = 1 << 1;
pub const PAGE_USER: u32 = 1 << 2;
pub const PAGE_PAGE_SIZE_4MB: u32 = 1 << 7;

const ENTRIES_PER_TABLE: usize = 1024;
const PAGE_SIZE_4K: u32 = 0x1000;
const TABLE_FLAGS: u32 = PAGE_PRESENT | PAGE_WRITABLE;
const PAGE_FRAME_MASK: u32 = 0xFFFF_F000;
const PAGE_TABLE_ALLOC_LIMIT: u64 = 0x0040_0000;

#[repr(C, align(4096))]
struct PageDirectory([u32; ENTRIES_PER_TABLE]);

#[repr(align(4096))]
struct PageTable([u32; ENTRIES_PER_TABLE]);

static mut BOOT_PAGE_DIRECTORY: PageDirectory = PageDirectory([0; ENTRIES_PER_TABLE]);
static mut BOOT_LOW_TABLE: PageTable = PageTable([0; ENTRIES_PER_TABLE]);
static mut BOOT_KERNEL_TABLE: PageTable = PageTable([0; ENTRIES_PER_TABLE]);
pub static mut PAGING_INITIALIZED: bool = false;

#[inline]
fn phys_to_virt(phys: u32) -> *mut u32 {
    (phys + KERNEL_SPACE_START as u32) as *mut u32
}

fn active_pd_ptr() -> *mut u32 {
    unsafe {
        if !PAGING_INITIALIZED {
            // During init_paging, force all mappings into the permanent static directory
            return (&raw mut BOOT_PAGE_DIRECTORY.0) as *mut u32;
        }
        // After boot, dynamically route to the hardware CR3 (for user processes)
        let cr3 = x86::read_cr3() & PAGE_FRAME_MASK;
        phys_to_virt(cr3)
    }
}

pub fn mark_paging_initialized() {
    unsafe { PAGING_INITIALIZED = true; }
}

fn kernel_pd_index() -> usize {
    (KERNEL_SPACE_START >> 22) as usize
}

#[inline]
const fn pde_index(virt_addr: u32) -> usize {
    (virt_addr >> 22) as usize
}

#[inline]
const fn pte_index(virt_addr: u32) -> usize {
    ((virt_addr >> 12) & 0x3FF) as usize
}

fn clear_page_directory() {
    let pd_ptr = (unsafe { &raw mut BOOT_PAGE_DIRECTORY.0 }) as *mut u32;
    for i in 0usize..ENTRIES_PER_TABLE {
        unsafe { pd_ptr.add(i).write(0) };
    }
}

fn fill_identity_low_table() {
    let pt_ptr = (unsafe { &raw mut BOOT_LOW_TABLE.0 }) as *mut u32;
    for i in 0usize..ENTRIES_PER_TABLE {
        let phys = (i as u32) * PAGE_SIZE_4K;
        unsafe { pt_ptr.add(i).write(phys | TABLE_FLAGS) };
    }
}

fn fill_kernel_low_alias_table() {
    let pt_ptr = (unsafe { &raw mut BOOT_KERNEL_TABLE.0 }) as *mut u32;
    for i in 0usize..ENTRIES_PER_TABLE {
        let phys = (i as u32) * PAGE_SIZE_4K;
        unsafe { pt_ptr.add(i).write(phys | TABLE_FLAGS) };
    }
}

fn zero_page_table(table_phys: u32) {
    let pt_ptr = phys_to_virt(table_phys);
    for i in 0usize..ENTRIES_PER_TABLE {
        unsafe { pt_ptr.add(i).write(0) };
    }
}

fn validate_virtual_address(virt_addr: u32, flags: u32) -> Result<(), &'static str> {
    if virt_addr == 0 {
        return Err("null page is not mappable");
    }

    if virt_addr >= KERNEL_SPACE_START as u32 {
        if (flags & PAGE_USER) != 0 {
            return Err("kernel mappings cannot be user-accessible");
        }
    } else {
        if virt_addr < USER_SPACE_START as u32 {
            return Err("address is below user space start");
        }
    }

    Ok(())
}

fn ensure_page_table(pde_index: usize) -> Result<u32, &'static str> {
    let pd_ptr = active_pd_ptr();
    let pde = unsafe { pd_ptr.add(pde_index).read() };

    if (pde & PAGE_PRESENT) != 0 {
        return Ok(pde & PAGE_FRAME_MASK);
    }

    let table_phys = physical::alloc_physical_page_below(PAGE_TABLE_ALLOC_LIMIT)
        .ok_or("no low physical frame available for page table")?;

    zero_page_table(table_phys);
    unsafe {
        pd_ptr
            .add(pde_index)
            .write(table_phys | TABLE_FLAGS | PAGE_USER)
    };
    x86::write_cr3(x86::read_cr3());

    Ok(table_phys)
}

fn get_page_entry_ptr(virt_addr: u32) -> Result<*mut u32, &'static str> {
    let pde = pde_index(virt_addr);
    let pt_phys = ensure_page_table(pde)?;
    let pt_ptr = phys_to_virt(pt_phys);
    Ok(unsafe { pt_ptr.add(pte_index(virt_addr)) })
}

fn lookup_page_entry_ptr(virt_addr: u32) -> Option<*mut u32> {
    unsafe {
        let pde = pde_index(virt_addr);
        let pd_ptr = active_pd_ptr();
        let pde_entry = pd_ptr.add(pde).read();
        if (pde_entry & PAGE_PRESENT) == 0 {
            return None;
        }
        let pt_phys = pde_entry & PAGE_FRAME_MASK;
        let pt_ptr = phys_to_virt(pt_phys);
        Some(pt_ptr.add(pte_index(virt_addr)))
    }
}

fn install_boot_mappings() {
    let pd_ptr = (unsafe { &raw mut BOOT_PAGE_DIRECTORY.0 }) as *mut u32;
    let low_table_phys = (unsafe { &raw const BOOT_LOW_TABLE.0 }) as *const u32 as u32;
    let kernel_table_phys = (unsafe { &raw const BOOT_KERNEL_TABLE.0 }) as *const u32 as u32;

    // Identity map first 4 MiB so current execution continues after PG=1.
    unsafe { pd_ptr.add(0).write(low_table_phys | TABLE_FLAGS) };

    // Map kernel higher-half base (3 GiB) to the same low 4 MiB for early transition.
    unsafe {
        pd_ptr
            .add(kernel_pd_index())
            .write(kernel_table_phys | TABLE_FLAGS)
    };
}

pub fn enable_bootstrap_paging() {
    unsafe {
        clear_page_directory();
        fill_identity_low_table();
        fill_kernel_low_alias_table();
        install_boot_mappings();

        let pd_phys = (&raw const BOOT_PAGE_DIRECTORY.0) as *const u32 as u32;
        x86::write_cr3(pd_phys);
        pr_debug!("Bootstrap paging tables loaded: cr3={:#x}\n", pd_phys);
    }

    x86::enable_paging();
    pr_debug!("CR0.PG set: paging is enabled\n");
}

pub fn bootstrap_directory_phys_addr() -> u32 {
    unsafe { (&raw const BOOT_PAGE_DIRECTORY.0) as *const u32 as u32 }
}

pub fn map_page_bootstrap(virt_addr: u32, phys_addr: u32, flags: u32) -> Result<(), &'static str> {
    let pde = pde_index(virt_addr);
    if pde != 0 && pde != kernel_pd_index() {
        pr_warn!(
            "map_page_bootstrap rejected unsupported pde={} va={:#x}\n",
            pde,
            virt_addr
        );
        return Err("bootstrap mapper only supports identity and higher-half PDE");
    }
    map_page(virt_addr, phys_addr, flags)
}

pub fn map_page(virt_addr: u32, phys_addr: u32, flags: u32) -> Result<(), &'static str> {
    if (virt_addr & !PAGE_FRAME_MASK) != 0 || (phys_addr & !PAGE_FRAME_MASK) != 0 {
        pr_warn!(
            "map_page rejected unaligned map va={:#x} pa={:#x}\n",
            virt_addr,
            phys_addr
        );
        return Err("addresses must be 4 KiB aligned");
    }

    validate_virtual_address(virt_addr, flags)?;

    unsafe {
        let entry_ptr = get_page_entry_ptr(virt_addr)?;
        entry_ptr.write((phys_addr & PAGE_FRAME_MASK) | (flags | PAGE_PRESENT));
        x86::invalidate_page(virt_addr);
    }

    pr_debug!(
        "map_page: va={:#x} -> pa={:#x} flags={:#x}\n",
        virt_addr,
        phys_addr,
        flags | PAGE_PRESENT
    );

    Ok(())
}

pub fn get_page_bootstrap(virt_addr: u32) -> Option<u32> {
    let pde = pde_index(virt_addr);
    if pde != 0 && pde != kernel_pd_index() {
        return None;
    }

    get_page(virt_addr)
}

pub fn get_page(virt_addr: u32) -> Option<u32> {
    if (virt_addr & !PAGE_FRAME_MASK) != 0 {
        return None;
    }

    unsafe {
        let pde = pde_index(virt_addr);
        let pd_ptr = (&raw const BOOT_PAGE_DIRECTORY.0) as *const u32;
        let pde_entry = pd_ptr.add(pde).read();
        if (pde_entry & PAGE_PRESENT) == 0 {
            return None;
        }

        let pt_ptr = (pde_entry & PAGE_FRAME_MASK) as *const u32;
        let entry = pt_ptr.add(pte_index(virt_addr)).read();
        if (entry & PAGE_PRESENT) == 0 {
            None
        } else {
            Some(entry)
        }
    }
}

#[allow(dead_code)]
pub fn unmap_page_bootstrap(virt_addr: u32) -> Result<(), &'static str> {
    let pde = pde_index(virt_addr);
    if pde != 0 && pde != kernel_pd_index() {
        pr_warn!(
            "unmap_page_bootstrap rejected unsupported pde={} va={:#x}\n",
            pde,
            virt_addr
        );
        return Err("bootstrap unmapper only supports identity and higher-half PDE");
    }

    unmap_page(virt_addr)
}

pub fn unmap_page(virt_addr: u32) -> Result<(), &'static str> {
    if (virt_addr & !PAGE_FRAME_MASK) != 0 {
        pr_warn!("unmap_page rejected unaligned va={:#x}\n", virt_addr);
        return Err("address must be 4 KiB aligned");
    }

    unsafe {
        let entry_ptr = match lookup_page_entry_ptr(virt_addr) {
            Some(ptr) => ptr,
            None => return Err("page table is not present"),
        };

        if (entry_ptr.read() & PAGE_PRESENT) == 0 {
            return Err("page is not mapped");
        }

        entry_ptr.write(0);
        x86::invalidate_page(virt_addr);
    }

    pr_debug!("unmap_page: va={:#x}\n", virt_addr);
    Ok(())
}

pub fn create_user_address_space() -> Option<u32> {
    unsafe {
        let pd_phys = physical::alloc_physical_page_below(PAGE_TABLE_ALLOC_LIMIT)?;
        zero_page_table(pd_phys);

        let new_pd_ptr = phys_to_virt(pd_phys);
        let boot_pd_ptr = (&raw const BOOT_PAGE_DIRECTORY.0) as *const u32;

        for i in 0..ENTRIES_PER_TABLE {
            new_pd_ptr.add(i).write(boot_pd_ptr.add(i).read());
        }

        Some(pd_phys)
    }
}
