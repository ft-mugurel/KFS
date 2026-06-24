use crate::paging::{self, PAGE_PRESENT, PAGE_USER, PAGE_WRITABLE};
use crate::sched::{ContextFrame, Vma, CURRENT_PID, MAX_VMAS, PROCESS_TABLE};
use crate::{pr_warn, x86};

pub(super) unsafe fn syscall_sbrk(regs: *mut ContextFrame) {
    unsafe {
        let increment = (*regs).arg1() as i32;
        let current_pid = CURRENT_PID;
        let task = PROCESS_TABLE[current_pid].as_mut().unwrap();

        let old_brk = task.memory.heap_brk;

        if increment == 0 {
            (*regs).set_return_value(old_brk);
            return;
        }

        if increment < 0 {
            pr_warn!("Shrinking the heap is not yet supported.\n");
            (*regs).set_return_value(!0u32); // Return -1
            return;
        }

        let new_brk = old_brk + increment as u32;
        let old_page_end = (old_brk + 4095) & !4095;

        if new_brk > old_page_end {
            let bytes_to_allocate = new_brk - old_page_end;
            let pages_to_allocate = (bytes_to_allocate + 4095) / 4096;

            for i in 0..pages_to_allocate {
                let vaddr = old_page_end + (i * 4096);

                if let Some(phys_frame) = paging::alloc_physical_page() {
                    let flags = PAGE_PRESENT | PAGE_WRITABLE | PAGE_USER;

                    paging::map_page(vaddr, phys_frame, flags).unwrap();
                } else {
                    pr_warn!("Out of physical memory for sbrk!\n");
                    (*regs).set_return_value(!0u32);
                    return;
                }
            }
        }

        task.memory.heap_brk = new_brk;
        (*regs).set_return_value(old_brk);
    }
}

pub(super) unsafe fn syscall_mmap(regs: *mut ContextFrame) {
    let length = (*regs).arg2() as usize;

    if length == 0 {
        (*regs).set_return_value(!0u32); // EINVAL
        return;
    }

    // Align length to the 4KB boundary
    let aligned_length = (length + 4095) & !4095;

    let current_pid = CURRENT_PID;
    let task = PROCESS_TABLE[current_pid].as_mut().unwrap();

    let mut vma_idx = None;
    for i in 0..MAX_VMAS {
        if !task.memory.vmas[i].used {
            vma_idx = Some(i);
            break;
        }
    }

    let vma_idx = match vma_idx {
        Some(idx) => idx,
        None => {
            (*regs).set_return_value(!0u32); // ENOMEM
            return;
        }
    };

    let mmap_base = 0x5000_0000 + (vma_idx as u32 * 0x100_000);
    /*
     *  Demand Paging:
     *      If a process requests a page but does not use it immediately or not even at all,
     *      we can delay the allocation until an access occurs.
     */

    task.memory.vmas[vma_idx] = Vma {
        base: mmap_base,
        size: aligned_length as u32,
        flags: 3, // PROT_READ | PROT_WRITE
        used: true,
    };

    (*regs).set_return_value(mmap_base);
}

pub(super) unsafe fn syscall_munmap(regs: *mut ContextFrame) {
    let addr = (*regs).arg1();
    let length = (*regs).arg2();

    if addr % 4096 != 0 || length == 0 {
        (*regs).set_return_value(!0u32); // EINVAL
        return;
    }

    let aligned_length = (length + 4095) & !4095;
    let current_pid = CURRENT_PID;
    let task = PROCESS_TABLE[current_pid].as_mut().unwrap();

    let mut target_vma_idx = None;
    for i in 0..MAX_VMAS {
        let vma = &task.memory.vmas[i];
        if vma.used && addr >= vma.base && (addr + aligned_length) <= (vma.base + vma.size) {
            target_vma_idx = Some(i);
            break;
        }
    }

    let vma_idx = match target_vma_idx {
        Some(idx) => idx,
        None => {
            (*regs).set_return_value(!0u32); // EINVAL (Not a mapped region)
            return;
        }
    };

    let old_cr3 = x86::read_cr3();
    x86::write_cr3(task.context.cr3);

    let num_pages = aligned_length / 4096;
    for i in 0..num_pages {
        let vaddr = addr + (i * 4096);
        if let Some(phys_frame) = paging::get_physical_address(vaddr) {
            paging::free_physical_page(phys_frame);
            paging::unmap_page(vaddr).unwrap_or_else(|_| {
                pr_warn!("Failed to unmap page at {:#x}\n", vaddr);
            });
        }
    }

    x86::write_cr3(old_cr3);

    let vma = &mut task.memory.vmas[vma_idx];
    if addr == vma.base && aligned_length == vma.size {
        vma.used = false;
    } else if addr == vma.base {
        vma.base += aligned_length;
        vma.size -= aligned_length;
    } else {
        vma.size -= aligned_length;
    }

    (*regs).set_return_value(0);
}
