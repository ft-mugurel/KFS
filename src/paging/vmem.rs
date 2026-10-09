use super::page_table;
use super::physical;
use super::{KERNEL_SPACE_START, PAGE_PCD, PAGE_SIZE, PAGE_WRITABLE};
use crate::error::{KResult, KernelError};
use crate::locks::Spinlock;
use crate::{pr_debug, pr_warn};

const MAX_ALLOCS: usize = 512;
const MAX_FREE_RANGES: usize = MAX_ALLOCS;

#[derive(Clone, Copy, Debug)]
struct FreeRange {
    start_page: usize,
    page_count: usize,
}

impl FreeRange {
    const EMPTY: Self = Self { start_page: 0, page_count: 0 };
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ArenaKind {
    Vmalloc,
    Ioremap,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AllocKind {
    RamBacked,
    IoRemap,
}

#[derive(Clone, Copy, Debug)]
pub struct VmemAlloc {
    pub base: u32,
    pub requested_size: usize,
    pub page_count: usize,
    pub total_pages: usize,
    pub kind: AllocKind,
    pub flags: u32,
    arena: ArenaKind,
}

#[derive(Clone, Copy, Debug)]
pub struct VmemStats {
    pub range_start: u32,
    pub range_end: u32,
    pub total_bytes: u32,
    pub free_bytes: u32,
    pub free_ranges: usize,
    pub alloc_count: usize,
    pub alloc_bytes: usize,
}

pub struct VmemState {
    arena_start: [u32; 2],
    arena_pages: [usize; 2],
    vmalloc_free: [FreeRange; MAX_FREE_RANGES],
    vmalloc_free_count: usize,
    ioremap_free: [FreeRange; MAX_FREE_RANGES],
    ioremap_free_count: usize,
    allocs: [Option<VmemAlloc>; MAX_ALLOCS],
    alloc_count: usize,
    total_allocated_bytes: usize,
}

impl VmemState {
    pub const fn new() -> Self {
        Self {
            arena_start: [0; 2],
            arena_pages: [0; 2],
            vmalloc_free: [FreeRange::EMPTY; MAX_FREE_RANGES],
            vmalloc_free_count: 0,
            ioremap_free: [FreeRange::EMPTY; MAX_FREE_RANGES],
            ioremap_free_count: 0,
            allocs: [None; MAX_ALLOCS],
            alloc_count: 0,
            total_allocated_bytes: 0,
        }
    }

    fn arena_index(arena: ArenaKind) -> usize {
        match arena {
            ArenaKind::Vmalloc => 0,
            ArenaKind::Ioremap => 1,
        }
    }

    fn arena_start(&self, arena: ArenaKind) -> u32 {
        self.arena_start[Self::arena_index(arena)]
    }

    fn arena_page_count(&self, arena: ArenaKind) -> usize {
        self.arena_pages[Self::arena_index(arena)]
    }

    fn free_ranges(&self, arena: ArenaKind) -> (&[FreeRange], usize) {
        match arena {
            ArenaKind::Vmalloc => (&self.vmalloc_free, self.vmalloc_free_count),
            ArenaKind::Ioremap => (&self.ioremap_free, self.ioremap_free_count),
        }
    }

    fn free_ranges_mut(&mut self, arena: ArenaKind) -> (&mut [FreeRange], &mut usize) {
        match arena {
            ArenaKind::Vmalloc => (&mut self.vmalloc_free, &mut self.vmalloc_free_count),
            ArenaKind::Ioremap => (&mut self.ioremap_free, &mut self.ioremap_free_count),
        }
    }

    fn reserve_pages(&mut self, arena: ArenaKind, count: usize) -> Option<usize> {
        let (ranges, range_count) = self.free_ranges_mut(arena);
        for index in 0..*range_count {
            let range = ranges[index];
            if range.page_count < count {
                continue;
            }
            let start = range.start_page;
            if range.page_count == count {
                for move_index in index..*range_count - 1 {
                    ranges[move_index] = ranges[move_index + 1];
                }
                *range_count -= 1;
            } else {
                ranges[index].start_page += count;
                ranges[index].page_count -= count;
            }
            return Some(start);
        }
        None
    }

    fn release_pages(&mut self, arena: ArenaKind, start_page: usize, page_count: usize) -> bool {
        let (ranges, range_count) = self.free_ranges_mut(arena);
        let mut insert = 0;
        while insert < *range_count && ranges[insert].start_page < start_page {
            insert += 1;
        }
        if *range_count >= MAX_FREE_RANGES {
            return false;
        }
        for index in (insert..*range_count).rev() {
            ranges[index + 1] = ranges[index];
        }
        ranges[insert] = FreeRange { start_page, page_count };
        *range_count += 1;

        let mut index = 0;
        while index + 1 < *range_count {
            let current = ranges[index];
            let next = ranges[index + 1];
            if current.start_page + current.page_count >= next.start_page {
                let end = (current.start_page + current.page_count)
                    .max(next.start_page + next.page_count);
                ranges[index].page_count = end - current.start_page;
                for move_index in index + 1..*range_count - 1 {
                    ranges[move_index] = ranges[move_index + 1];
                }
                *range_count -= 1;
            } else {
                index += 1;
            }
        }
        true
    }

    pub fn find_free_slot(&self) -> Option<usize> {
        for (i, alloc) in self.allocs.iter().enumerate() {
            if alloc.is_none() {
                return Some(i);
            }
        }
        None
    }

    pub fn find_slot_by_ptr(&self, ptr: u32) -> Option<usize> {
        let page_aligned = ptr & !(PAGE_SIZE as u32 - 1);
        for (i, alloc_opt) in self.allocs.iter().enumerate() {
            if let Some(alloc) = alloc_opt {
                if alloc.base == ptr || alloc.base == page_aligned {
                    return Some(i);
                }
            }
        }
        None
    }

    fn count_free_pages(&self, arena: ArenaKind) -> usize {
        let (ranges, count) = self.free_ranges(arena);
        ranges[..count].iter().map(|range| range.page_count).sum()
    }

    fn count_free_ranges(&self, arena: ArenaKind) -> usize {
        self.free_ranges(arena).1
    }
}

static VMEM_STATE: Spinlock<VmemState> = Spinlock::new(VmemState::new());

fn page_count_for(size: usize) -> KResult<usize> {
    size.checked_add(PAGE_SIZE - 1)
        .map(|aligned_size| aligned_size / PAGE_SIZE)
        .ok_or(KernelError::EOVERFLOW)
}

fn validate_vmalloc_flags(flags: u32) -> KResult<()> {
    if (flags & !(PAGE_WRITABLE | PAGE_PCD)) != 0 {
        return Err(KernelError::EINVAL);
    }
    Ok(())
}

fn validate_ioremap_flags(flags: u32) -> KResult<()> {
    if (flags & !(PAGE_WRITABLE | PAGE_PCD)) != 0 || (flags & PAGE_PCD) == 0 {
        return Err(KernelError::EINVAL);
    }
    Ok(())
}

#[unsafe(link_section = ".init.text")]
pub fn init_vmem() {
    let mut state = VMEM_STATE.lock();
    let address_space_start = KERNEL_SPACE_START as u32 + 0x1000_0000;
    let address_space_end = super::kernel_heap::virtual_start();
    let address_space_pages = (address_space_end - address_space_start) as usize / PAGE_SIZE;
    let normal_pages = address_space_pages / 2;
    let io_pages = address_space_pages - normal_pages;
    state.arena_start = [
        address_space_start,
        address_space_start + (normal_pages * PAGE_SIZE) as u32,
    ];
    state.arena_pages = [normal_pages, io_pages];
    state.vmalloc_free.fill(FreeRange::EMPTY);
    state.ioremap_free.fill(FreeRange::EMPTY);
    state.vmalloc_free[0] = FreeRange { start_page: 0, page_count: normal_pages };
    state.ioremap_free[0] = FreeRange { start_page: 0, page_count: io_pages };
    state.vmalloc_free_count = 1;
    state.ioremap_free_count = 1;
    state.allocs.fill(None);
    state.alloc_count = 0;
    state.total_allocated_bytes = 0;
    pr_debug!(
        "vmem initialized: range=[{:#x}, {:#x}) total_pages={} max_allocs={}\n",
        address_space_start,
        address_space_end,
        address_space_pages,
        MAX_ALLOCS
    );
}

#[inline(never)]
pub fn vmalloc(size: usize) -> KResult<*mut u8> {
    vmalloc_with_flags(size, PAGE_WRITABLE)
}

#[inline(never)]
pub fn vmalloc_with_flags(size: usize, flags: u32) -> KResult<*mut u8> {
    if size == 0 {
        return Err(KernelError::EINVAL);
    }
    validate_vmalloc_flags(flags)?;
    let mut state = VMEM_STATE.lock();

    let page_count = match page_count_for(size) {
        Ok(count) => count,
        Err(error) => {
            pr_warn!("vmalloc size overflow size={}\n", size);
            return Err(error);
        }
    };
    if page_count == 0 || page_count >= state.arena_page_count(ArenaKind::Vmalloc) {
        pr_warn!(
            "vmalloc request too large size={} pages={} max_pages={}\n",
            size,
            page_count,
            state.arena_page_count(ArenaKind::Vmalloc) - 1
        );
        return Err(KernelError::EINVAL);
    }

    // Redzone Guard Page: reserve 1 extra trailing page that remains unmapped.
    let total_pages = match page_count.checked_add(1) {
        Some(v) if v <= state.arena_page_count(ArenaKind::Vmalloc) => v,
        _ => {
            pr_warn!(
                "vmalloc page count overflow size={} pages={}\n",
                size,
                page_count
            );
            return Err(KernelError::EOVERFLOW);
        }
    };

    let slot = match state.find_free_slot() {
        Some(v) => v,
        None => {
            pr_warn!("vmalloc allocation table full\n");
            return Err(KernelError::ENOMEM);
        }
    };

    let start_page = match state.reserve_pages(ArenaKind::Vmalloc, total_pages) {
        Some(v) => v,
        None => {
            pr_warn!(
                "vmalloc out of virtual space size={} pages={} total_pages={}\n",
                size,
                page_count,
                total_pages
            );
            return Err(KernelError::ENOMEM);
        }
    };

    let base = state.arena_start(ArenaKind::Vmalloc) + (start_page * PAGE_SIZE) as u32;

    // Map physical frames only for the requested data pages.
    // The trailing guard page is left unmapped for hardware buffer overrun detection.
    for mapped_pages in 0..page_count {
        let frame = match physical::alloc_physical_page() {
            Ok(f) => f,
            Err(e) => {
                pr_warn!(
                    "vmalloc ran out of physical pages at mapped_pages={}\n",
                    mapped_pages
                );
                for j in 0..mapped_pages {
                    let va = base + (j * PAGE_SIZE) as u32;
                    if let Some(entry) = page_table::get_page(va) {
                        let f = entry & 0xFFFF_F000;
                        let _ = page_table::unmap_page(va);
                        let _ = physical::free_physical_page(f);
                    }
                }
                state.release_pages(ArenaKind::Vmalloc, start_page, total_pages);
                return Err(e);
            }
        };

        let va = base + (mapped_pages * PAGE_SIZE) as u32;
        if page_table::map_page(va, frame, flags).is_err() {
            let _ = physical::free_physical_page(frame);
            for j in 0..mapped_pages {
                let prev_va = base + (j * PAGE_SIZE) as u32;
                if let Some(entry) = page_table::get_page(prev_va) {
                    let f = entry & 0xFFFF_F000;
                    let _ = page_table::unmap_page(prev_va);
                    let _ = physical::free_physical_page(f);
                }
            }
            state.release_pages(ArenaKind::Vmalloc, start_page, total_pages);
            return Err(KernelError::ENOMEM);
        }
    }

    state.allocs[slot] = Some(VmemAlloc {
        base,
        requested_size: size,
        page_count,
        total_pages,
        kind: AllocKind::RamBacked,
        flags,
        arena: ArenaKind::Vmalloc,
    });
    state.alloc_count += 1;
    state.total_allocated_bytes += size;

    pr_debug!(
        "vmalloc size={} pages={} total_pages={} base={:#x} end={:#x}\n",
        size,
        page_count,
        total_pages,
        base,
        base + (page_count * PAGE_SIZE) as u32
    );

    Ok(base as *mut u8)
}

#[inline(never)]
pub fn vfree(ptr: *mut u8) -> KResult<()> {
    let addr = ptr as u32;
    if addr == 0 {
        pr_warn!("vfree rejected null pointer\n");
        return Err(KernelError::EINVAL);
    }
    let mut state = VMEM_STATE.lock();
    let address_start = state.arena_start(ArenaKind::Vmalloc);
    let address_end = state.arena_start(ArenaKind::Ioremap)
        + (state.arena_page_count(ArenaKind::Ioremap) * PAGE_SIZE) as u32;
    if addr < address_start || addr >= address_end {
        pr_warn!("vfree pointer outside vmem range ptr={:#x}\n", addr);
        return Err(KernelError::EFAULT);
    }
    let slot = match state.find_slot_by_ptr(addr) {
        Some(v) => v,
        None => {
            pr_warn!("vfree unknown pointer={:#x}\n", addr);
            return Err(KernelError::EFAULT);
        }
    };

    let alloc = match state.allocs[slot] {
        Some(a) => a,
        None => return Err(KernelError::EFAULT),
    };

    match alloc.kind {
        AllocKind::RamBacked => {
            for i in 0..alloc.page_count {
                let va = alloc.base + (i * PAGE_SIZE) as u32;
                if let Some(entry) = page_table::get_page(va) {
                    let frame = entry & 0xFFFF_F000;
                    let _ = page_table::unmap_page(va);
                    let _ = physical::free_physical_page(frame);
                }
            }
        }
        AllocKind::IoRemap => {
            for i in 0..alloc.page_count {
                let va = alloc.base + (i * PAGE_SIZE) as u32;
                let _ = page_table::unmap_page(va);
            }
        }
    }

    let start_page = ((alloc.base - state.arena_start(alloc.arena)) as usize) / PAGE_SIZE;
    state.release_pages(alloc.arena, start_page, alloc.total_pages);

    state.total_allocated_bytes = state
        .total_allocated_bytes
        .saturating_sub(alloc.requested_size);
    state.alloc_count = state.alloc_count.saturating_sub(1);
    state.allocs[slot] = None;

    pr_debug!(
        "vfree ptr={:#x} size={} pages={}\n",
        alloc.base,
        alloc.requested_size,
        alloc.page_count
    );

    Ok(())
}

#[inline(never)]
pub fn vsize(ptr: *const u8) -> KResult<usize> {
    let addr = ptr as u32;
    if addr == 0 {
        return Err(KernelError::EINVAL);
    }
    let state = VMEM_STATE.lock();
    let address_start = state.arena_start(ArenaKind::Vmalloc);
    let address_end = state.arena_start(ArenaKind::Ioremap)
        + (state.arena_page_count(ArenaKind::Ioremap) * PAGE_SIZE) as u32;
    if addr < address_start || addr >= address_end {
        return Err(KernelError::EFAULT);
    }
    if let Some(slot) = state.find_slot_by_ptr(addr) {
        if let Some(alloc) = state.allocs[slot] {
            return Ok(alloc.requested_size);
        }
    }

    Err(KernelError::EFAULT)
}

#[inline(never)]
pub fn ioremap(phys_addr: u32, size: usize) -> KResult<*mut u8> {
    ioremap_with_flags(phys_addr, size, PAGE_WRITABLE | PAGE_PCD)
}

#[inline(never)]
pub fn ioremap_with_flags(phys_addr: u32, size: usize, flags: u32) -> KResult<*mut u8> {
    if size == 0 {
        return Err(KernelError::EINVAL);
    }
    validate_ioremap_flags(flags)?;
    let mut state = VMEM_STATE.lock();

    let offset = (phys_addr as usize) & (PAGE_SIZE - 1);
    let aligned_phys = phys_addr & !(PAGE_SIZE as u32 - 1);
    let aligned_size = size.checked_add(offset).ok_or(KernelError::EOVERFLOW)?;
    let page_count = match page_count_for(aligned_size) {
        Ok(count) => count,
        Err(error) => {
            pr_warn!(
                "ioremap aligned size overflow phys={:#x} size={}\n",
                phys_addr,
                size
            );
            return Err(error);
        }
    };

    if page_count == 0 || page_count >= state.arena_page_count(ArenaKind::Ioremap) {
        return Err(KernelError::EINVAL);
    }

    let total_pages = page_count.checked_add(1).ok_or(KernelError::EOVERFLOW)?;
    if total_pages > state.arena_page_count(ArenaKind::Ioremap) {
        return Err(KernelError::ENOMEM);
    }

    let slot = match state.find_free_slot() {
        Some(v) => v,
        None => {
            pr_warn!("ioremap allocation table full\n");
            return Err(KernelError::ENOMEM);
        }
    };

    let start_page = match state.reserve_pages(ArenaKind::Ioremap, total_pages) {
        Some(v) => v,
        None => {
            pr_warn!("ioremap out of virtual space size={}\n", size);
            return Err(KernelError::ENOMEM);
        }
    };

    let base = state.arena_start(ArenaKind::Ioremap) + (start_page * PAGE_SIZE) as u32;
    for i in 0..page_count {
        let va = base + (i * PAGE_SIZE) as u32;
        let pa = match aligned_phys.checked_add((i * PAGE_SIZE) as u32) {
            Some(address) => address,
            None => {
                for j in 0..i {
                    let mapped_va = base + (j * PAGE_SIZE) as u32;
                    let _ = page_table::unmap_page(mapped_va);
                }
                state.release_pages(ArenaKind::Ioremap, start_page, total_pages);
                pr_warn!(
                    "ioremap physical range overflow phys={:#x} pages={}\n",
                    phys_addr,
                    page_count
                );
                return Err(KernelError::EOVERFLOW);
            }
        };
        if page_table::map_page(va, pa, flags).is_err() {
            for j in 0..i {
                let mapped_va = base + (j * PAGE_SIZE) as u32;
                let _ = page_table::unmap_page(mapped_va);
            }
            state.release_pages(ArenaKind::Ioremap, start_page, total_pages);
            return Err(KernelError::ENOMEM);
        }
    }

    state.allocs[slot] = Some(VmemAlloc {
        base,
        requested_size: size,
        page_count,
        total_pages,
        kind: AllocKind::IoRemap,
        flags,
        arena: ArenaKind::Ioremap,
    });
    state.alloc_count += 1;
    state.total_allocated_bytes += size;

    pr_debug!(
        "ioremap phys={:#x} size={} virt={:#x}\n",
        phys_addr,
        size,
        base + offset as u32
    );

    Ok((base + offset as u32) as *mut u8)
}

#[inline(never)]
pub fn iounmap(ptr: *mut u8) -> KResult<()> {
    vfree(ptr)
}

pub fn debug_stats() -> VmemStats {
    let state = VMEM_STATE.lock();
    let free_bytes = (state.count_free_pages(ArenaKind::Vmalloc)
        + state.count_free_pages(ArenaKind::Ioremap))
        * PAGE_SIZE;
    let free_ranges =
        state.count_free_ranges(ArenaKind::Vmalloc) + state.count_free_ranges(ArenaKind::Ioremap);
    let range_start = state.arena_start(ArenaKind::Vmalloc);
    let range_end = state.arena_start(ArenaKind::Ioremap)
        + (state.arena_page_count(ArenaKind::Ioremap) * PAGE_SIZE) as u32;

    VmemStats {
        range_start,
        range_end,
        total_bytes: range_end - range_start,
        free_bytes: free_bytes as u32,
        free_ranges,
        alloc_count: state.alloc_count,
        alloc_bytes: state.total_allocated_bytes,
    }
}

pub fn debug_for_each_alloc(mut f: impl FnMut(u32, usize, usize)) {
    let state = VMEM_STATE.lock();
    for alloc in state.allocs.iter().flatten() {
        f(alloc.base, alloc.requested_size, alloc.page_count);
    }
}

pub fn debug_for_each_free_range(mut f: impl FnMut(u32, u32)) {
    let state = VMEM_STATE.lock();
    for arena in [ArenaKind::Vmalloc, ArenaKind::Ioremap] {
        let (ranges, count) = state.free_ranges(arena);
        for range in &ranges[..count] {
            f(
                state.arena_start(arena) + (range.start_page * PAGE_SIZE) as u32,
                (range.page_count * PAGE_SIZE) as u32,
            );
        }
    }
}
