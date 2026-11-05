use super::args::{UserAddr, syscall_define};
use crate::error::KernelError;
use crate::paging::{self, PAGE_PRESENT, PAGE_USER, PAGE_WRITABLE};
use crate::sched::{self, MAX_VMAS, TaskStruct, Vma};
use crate::{pr_warn, x86};

const MMAP_BASE: u32 = 0x5000_0000;
/// Each VMA index owns one slot, so a mapping must fit inside it.
const MMAP_SLOT_SIZE: u32 = 0x10_0000;

/// The calling task.
///
/// The `&mut` is only sound while nothing else touches this task: the
/// handlers below keep it for their whole body and never block or take
/// PROCESS_TABLE.
fn current_task<'a>() -> Result<&'a mut TaskStruct, KernelError> {
    unsafe { sched::current().as_mut() }.ok_or(KernelError::Srch)
}

syscall_define! {
    pub(super) fn syscall_sbrk(increment: i32) {
        let mem = &mut current_task()?.memory;
        let old_brk = mem.heap_brk;

        if increment == 0 {
            return Ok(old_brk);
        }
        if increment < 0 {
            pr_warn!("Shrinking the heap is not yet supported.");
            return Err(KernelError::Inval);
        }

        // The heap must stay in user space, or it would map user pages over
        // the kernel half
        let new_brk = old_brk
            .checked_add(increment as u32)
            .filter(|&brk| brk as usize <= paging::KERNEL_SPACE_START)
            .ok_or(KernelError::NoMem)?;
        let old_page_end = (old_brk + 4095) & !4095;

        if new_brk > old_page_end {
            let pages_to_allocate = (new_brk - old_page_end).div_ceil(4096);
            for i in 0..pages_to_allocate {
                let vaddr = old_page_end + (i * 4096);
                let Ok(phys_frame) = paging::alloc_physical_page_below(paging::PAGE_TABLE_ALLOC_LIMIT) else {
                    pr_warn!("Out of physical memory for sbrk!");
                    return Err(KernelError::NoMem);
                };
                paging::map_page(vaddr, phys_frame, PAGE_PRESENT | PAGE_WRITABLE | PAGE_USER).unwrap();
            }
        }

        mem.heap_brk = new_brk;
        Ok(old_brk)
    }
}

syscall_define! {
    /// mmap(hint, length): reserves a read/write area that is filled in on
    /// first access (demand paging). The hint is ignored.
    pub(super) fn syscall_mmap(_hint: UserAddr, length: usize) {
        if length == 0 {
            return Err(KernelError::Inval);
        }
        let aligned_length = length.checked_next_multiple_of(4096).ok_or(KernelError::NoMem)?;
        if aligned_length > MMAP_SLOT_SIZE as usize {
            return Err(KernelError::NoMem);
        }

        let mem = &mut current_task()?.memory;
        let vma_idx = (0..MAX_VMAS).find(|&i| !mem.vmas[i].used).ok_or(KernelError::NoMem)?;
        let mmap_base = MMAP_BASE + vma_idx as u32 * MMAP_SLOT_SIZE;

        mem.vmas[vma_idx] = Vma {
            base: mmap_base,
            size: aligned_length as u32,
            // The user ABI passes no prot argument; mappings are always RW.
            flags: sched::VMA_READ | sched::VMA_WRITE,
            used: true,
        };
        Ok(mmap_base)
    }
}

syscall_define! {
    pub(super) fn syscall_munmap(addr: u32, length: u32) {
        if !addr.is_multiple_of(4096) || length == 0 {
            return Err(KernelError::Inval);
        }
        let aligned_length = length.checked_next_multiple_of(4096).ok_or(KernelError::Inval)?;
        let end = addr.checked_add(aligned_length).ok_or(KernelError::Inval)?;

        let task = current_task()?;
        let vma_idx = (0..MAX_VMAS)
            .find(|&i| {
                let vma = &task.memory.vmas[i];
                vma.used && addr >= vma.base && end <= vma.base + vma.size
            })
            .ok_or(KernelError::Inval)?;

        let mut result = Ok(0);
        x86::disable_interrupts();
        let old_cr3 = x86::read_cr3();
        unsafe { x86::write_cr3(task.context.cr3) };

        for i in 0..aligned_length / 4096 {
            let vaddr = addr + (i * 4096);
            if let Some(phys_frame) = unsafe { paging::virt_to_phys(vaddr) } {
                if let Err(e) = paging::free_physical_page(phys_frame) {
                    pr_warn!("Failed to free physical page: {:?}", e);
                    result = Err(e);
                }
                if let Err(e) = paging::unmap_page(vaddr) {
                    pr_warn!("Failed to unmap page at {:#x}: {:?}", vaddr, e);
                    result = Err(e);
                }
            }
        }

        unsafe { x86::write_cr3(old_cr3) };
        unsafe { x86::enable_interrupts() };

        let vma = &mut task.memory.vmas[vma_idx];
        if addr == vma.base && aligned_length == vma.size {
            vma.used = false;
        } else if addr == vma.base {
            vma.base += aligned_length;
            vma.size -= aligned_length;
        } else {
            vma.size -= aligned_length;
        }
        result
    }
}
