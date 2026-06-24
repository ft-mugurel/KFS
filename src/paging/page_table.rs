use super::init::{KERNEL_SPACE_START, USER_SPACE_START};
use super::physical;
use super::{PAGE_PRESENT, PAGE_USER, PAGE_WRITABLE};
use crate::error::{KResult, KernelError};
use crate::{pr_debug, pr_warn, x86};

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
static mut PAGING_INITIALIZED: bool = false;

#[inline]
pub fn phys_to_virt(phys: u32) -> *mut u32 {
    (phys + KERNEL_SPACE_START as u32) as *mut u32
}

pub unsafe fn get_physical_address(vaddr: u32) -> Option<u32> {
    let pd_phys = x86::read_cr3();
    let pd_virt = phys_to_virt(pd_phys) as *const u32;

    let pde_idx = (vaddr >> 22) as usize;
    let pte_idx = ((vaddr >> 12) & 0x3FF) as usize;

    let pde = pd_virt.add(pde_idx).read();
    if (pde & PAGE_PRESENT) == 0 {
        return None;
    }

    let pt_phys = pde & PAGE_FRAME_MASK;
    let pt_virt = phys_to_virt(pt_phys) as *const u32;

    let pte = pt_virt.add(pte_idx).read();
    if (pte & PAGE_PRESENT) == 0 {
        return None;
    }

    Some(pte & PAGE_FRAME_MASK)
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
    unsafe {
        PAGING_INITIALIZED = true;
    }
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

pub fn zero_page_table(table_phys: u32) {
    let pt_ptr = phys_to_virt(table_phys);
    for i in 0usize..ENTRIES_PER_TABLE {
        unsafe { pt_ptr.add(i).write(0) };
    }
}

fn validate_virtual_address(virt_addr: u32, flags: u32) -> KResult<()> {
    if virt_addr == 0 {
        return Err(KernelError::EFAULT);
    }

    if virt_addr >= KERNEL_SPACE_START as u32 {
        if (flags & PAGE_USER) != 0 {
            return Err(KernelError::EPERM);
        }
    } else {
        if virt_addr < USER_SPACE_START as u32 {
            return Err(KernelError::EFAULT);
        }
    }

    Ok(())
}

fn ensure_page_table(pde_index: usize) -> KResult<u32> {
    let pd_ptr = active_pd_ptr();
    let pde = unsafe { pd_ptr.add(pde_index).read() };

    if (pde & PAGE_PRESENT) != 0 {
        return Ok(pde & PAGE_FRAME_MASK);
    }

    let table_phys =
        physical::alloc_physical_page_below(PAGE_TABLE_ALLOC_LIMIT).ok_or(KernelError::ENOMEM)?;

    zero_page_table(table_phys);
    unsafe {
        pd_ptr
            .add(pde_index)
            .write(table_phys | TABLE_FLAGS | PAGE_USER)
    };
    x86::write_cr3(x86::read_cr3());

    Ok(table_phys)
}

fn get_page_entry_ptr(virt_addr: u32) -> KResult<*mut u32> {
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

pub fn map_page_bootstrap(virt_addr: u32, phys_addr: u32, flags: u32) -> KResult<()> {
    let pde = pde_index(virt_addr);
    if pde != 0 && pde != kernel_pd_index() {
        pr_warn!(
            "map_page_bootstrap rejected unsupported pde={} va={:#x}\n",
            pde,
            virt_addr
        );
        return Err(KernelError::EOPNOTSUPP);
    }
    map_page(virt_addr, phys_addr, flags)
}

pub fn map_page(virt_addr: u32, phys_addr: u32, flags: u32) -> KResult<()> {
    if (virt_addr & !PAGE_FRAME_MASK) != 0 || (phys_addr & !PAGE_FRAME_MASK) != 0 {
        pr_warn!(
            "map_page rejected unaligned map va={:#x} pa={:#x}\n",
            virt_addr,
            phys_addr
        );
        return Err(KernelError::EINVAL);
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

pub fn map_zero_page(virt_addr: u32, flags: u32) -> KResult<()> {
    let phys_frame = physical::alloc_physical_page().ok_or(KernelError::ENOMEM)?;
    unsafe {
        core::ptr::write_bytes(phys_to_virt(phys_frame) as *mut u8, 0, 4096);
    }
    map_page(virt_addr, phys_frame, flags)
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
pub fn unmap_page_bootstrap(virt_addr: u32) -> KResult<()> {
    let pde = pde_index(virt_addr);
    if pde != 0 && pde != kernel_pd_index() {
        pr_warn!(
            "unmap_page_bootstrap rejected unsupported pde={} va={:#x}\n",
            pde,
            virt_addr
        );
        return Err(KernelError::EOPNOTSUPP);
    }

    unmap_page(virt_addr)
}

pub fn unmap_page(virt_addr: u32) -> KResult<()> {
    if (virt_addr & !PAGE_FRAME_MASK) != 0 {
        pr_warn!("unmap_page rejected unaligned va={:#x}\n", virt_addr);
        return Err(KernelError::EINVAL);
    }

    unsafe {
        let entry_ptr = match lookup_page_entry_ptr(virt_addr) {
            Some(ptr) => ptr,
            None => return Err(KernelError::EFAULT),
        };

        if (entry_ptr.read() & PAGE_PRESENT) == 0 {
            return Err(KernelError::EFAULT);
        }

        entry_ptr.write(0);
        x86::invalidate_page(virt_addr);
    }

    pr_debug!("unmap_page: va={:#x}\n", virt_addr);
    Ok(())
}

pub unsafe fn clone_address_space(parent_cr3: u32) -> Option<u32> {
    let child_pd_phys = physical::alloc_physical_page()?;
    let child_pd = phys_to_virt(child_pd_phys) as *mut u32;
    let parent_pd = phys_to_virt(parent_cr3) as *const u32;

    // We just copy the pointers so kernel memory is shared natively.
    core::ptr::copy_nonoverlapping(parent_pd.add(768), child_pd.add(768), 256);

    // Deep Copy User Space
    for pde_idx in 0..768 {
        let pde = parent_pd.add(pde_idx).read();

        if (pde & PAGE_PRESENT) != 0 && (pde & PAGE_USER) != 0 {
            let child_pt_phys = physical::alloc_physical_page()?;
            let child_pt = phys_to_virt(child_pt_phys) as *mut u32;
            let parent_pt = phys_to_virt(pde & PAGE_FRAME_MASK) as *const u32;

            core::ptr::write_bytes(child_pt, 0, 1024);
            child_pd.add(pde_idx).write(child_pt_phys | (pde & 0xFFF)); // Preserve original flags

            for pte_idx in 0..1024 {
                let pte = parent_pt.add(pte_idx).read();

                if (pte & PAGE_PRESENT) != 0 && (pte & PAGE_USER) != 0 {
                    // Allocate a new physical frame for the actual data
                    let data_phys = physical::alloc_physical_page()?;
                    let data_virt_child = phys_to_virt(data_phys) as *mut u8;
                    let data_virt_parent = phys_to_virt(pte & PAGE_FRAME_MASK) as *const u8;

                    core::ptr::copy_nonoverlapping(data_virt_parent, data_virt_child, 4096);

                    child_pt.add(pte_idx).write(data_phys | (pte & 0xFFF));
                } else if (pte & PAGE_PRESENT) != 0 {
                    // Present but Kernel-owned
                    child_pt.add(pte_idx).write(pte);
                }
            }
        } else if (pde & PAGE_PRESENT) != 0 {
            child_pd.add(pde_idx).write(pde);
        } else {
            child_pd.add(pde_idx).write(0);
        }
    }

    Some(child_pd_phys)
}

pub unsafe fn free_user_address_space(cr3: u32) {
    let pd_virt: *mut u32 = phys_to_virt(cr3) as *mut u32;

    for pde_idx in 0..768 {
        let pde = pd_virt.add(pde_idx).read();

        if (pde & PAGE_PRESENT) != 0 && (pde & PAGE_USER) != 0 {
            let pt_phys = pde & PAGE_FRAME_MASK;
            let pt_virt = phys_to_virt(pt_phys) as *mut u32;

            for pte_idx in 0..1024 {
                let pte = pt_virt.add(pte_idx).read();

                if (pte & PAGE_PRESENT) != 0 && (pte & PAGE_USER) != 0 {
                    let data_phys = pte & PAGE_FRAME_MASK;
                    physical::free_physical_page(data_phys);
                }
            }

            physical::free_physical_page(pt_phys);
        }
    }

    physical::free_physical_page(cr3);
}
