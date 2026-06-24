use super::init::{KERNEL_SPACE_START, PAGE_SIZE};
use super::page_table;
use super::physical;
use super::PAGE_WRITABLE;
use crate::error::{KResult, KernelError};
use crate::{pr_debug, pr_warn};
use core::mem::size_of;
use core::ptr;

const HEAP_ALIGNMENT: usize = 16;
const HEAP_MAGIC: u32 = 0x4B_48_45_50;
const BLOCK_MAGIC: u32 = 0x4B_42_4C_4B;
const HEAP_CHUNK_GRANULARITY: usize = PAGE_SIZE;
const HEAP_DEFAULT_CHUNK_SIZE: usize = HEAP_CHUNK_GRANULARITY * 4;
const HEAP_VM_START: u32 = KERNEL_SPACE_START as u32 + 0x0100_0000;
const HEAP_VM_END: u32 = KERNEL_SPACE_START as u32 + 0x0200_0000;
const HEAP_MAX_FREE_RANGES: usize = 128;

#[derive(Clone, Copy)]
struct HeapFreeRange {
    base: u32,
    size: u32,
}

#[repr(C, align(16))]
struct HeapChunk {
    magic: u32,
    total_size: usize,
    prev: *mut HeapChunk,
    next: *mut HeapChunk,
}

#[repr(C, align(16))]
struct BlockHeader {
    magic: u32,
    total_size: usize,
    usable_size: usize,
    requested_size: usize,
    chunk: *mut HeapChunk,
    prev_free: *mut BlockHeader,
    next_free: *mut BlockHeader,
    free: u32,
}

#[repr(C, align(16))]
struct BlockFooter {
    magic: u32,
    total_size: usize,
}

#[derive(Clone, Copy)]
pub struct HeapStats {
    pub ready: bool,
    pub chunk_count: usize,
    pub chunk_bytes: usize,
    pub free_block_count: usize,
    pub free_bytes: usize,
    pub used_block_count: usize,
    pub used_requested_bytes: usize,
    pub used_usable_bytes: usize,
}

static mut HEAP_READY: bool = false;
static mut HEAP_CHUNKS: *mut HeapChunk = ptr::null_mut();
static mut HEAP_FREE_LIST: *mut BlockHeader = ptr::null_mut();
static mut HEAP_FREE_RANGES: [HeapFreeRange; HEAP_MAX_FREE_RANGES] =
    [HeapFreeRange { base: 0, size: 0 }; HEAP_MAX_FREE_RANGES];
static mut HEAP_FREE_RANGE_COUNT: usize = 0;

fn align_up(value: usize, align: usize) -> Option<usize> {
    if align == 0 || !align.is_power_of_two() {
        return None;
    }

    let mask = align - 1;
    value.checked_add(mask).map(|rounded| rounded & !mask)
}

fn block_header_size() -> usize {
    size_of::<BlockHeader>()
}

fn block_footer_size() -> usize {
    size_of::<BlockFooter>()
}

fn chunk_header_size() -> usize {
    size_of::<HeapChunk>()
}

fn block_alignment_overhead() -> usize {
    block_header_size() + block_footer_size()
}

fn minimum_free_block_size() -> usize {
    block_alignment_overhead() + HEAP_ALIGNMENT
}

fn chunk_alloc_size(min_block_total_size: usize) -> Option<usize> {
    let min_chunk_size = chunk_header_size().checked_add(min_block_total_size)?;
    let requested = core::cmp::max(min_chunk_size, HEAP_DEFAULT_CHUNK_SIZE);
    align_up(requested, HEAP_CHUNK_GRANULARITY)
}

fn pages_for(size: usize) -> Option<usize> {
    size.checked_add(PAGE_SIZE - 1)
        .map(|v| v / PAGE_SIZE)
        .filter(|pages| *pages > 0)
}

fn pages_to_bytes(pages: usize) -> Option<u32> {
    let bytes = pages.checked_mul(PAGE_SIZE)?;
    if bytes > u32::MAX as usize {
        None
    } else {
        Some(bytes as u32)
    }
}

fn allocate_heap_virtual_span(span: u32) -> Option<u32> {
    unsafe {
        for i in 0usize..HEAP_FREE_RANGE_COUNT {
            let range = HEAP_FREE_RANGES[i];
            if range.size >= span {
                let base = range.base;
                let remaining = range.size - span;
                if remaining == 0 {
                    for j in i..(HEAP_FREE_RANGE_COUNT - 1) {
                        HEAP_FREE_RANGES[j] = HEAP_FREE_RANGES[j + 1];
                    }
                    HEAP_FREE_RANGE_COUNT -= 1;
                } else {
                    HEAP_FREE_RANGES[i] =
                        HeapFreeRange { base: range.base + span, size: remaining };
                }
                return Some(base);
            }
        }
    }

    None
}

fn insert_heap_virtual_span(mut base: u32, mut size: u32) -> bool {
    if size == 0 {
        return true;
    }

    let end = match base.checked_add(size) {
        Some(v) => v,
        None => return false,
    };

    let mut i = 0usize;
    unsafe {
        while i < HEAP_FREE_RANGE_COUNT {
            let range = HEAP_FREE_RANGES[i];
            let range_end = match range.base.checked_add(range.size) {
                Some(v) => v,
                None => return false,
            };

            if end == range.base {
                base = range.base;
                size = size.saturating_add(range.size);
                let mut j = i;
                while j + 1 < HEAP_FREE_RANGE_COUNT {
                    HEAP_FREE_RANGES[j] = HEAP_FREE_RANGES[j + 1];
                    j += 1;
                }
                HEAP_FREE_RANGE_COUNT -= 1;
                i = 0;
                continue;
            }

            if range_end == base {
                base = range.base;
                size = size.saturating_add(range.size);
                let mut j = i;
                while j + 1 < HEAP_FREE_RANGE_COUNT {
                    HEAP_FREE_RANGES[j] = HEAP_FREE_RANGES[j + 1];
                    j += 1;
                }
                HEAP_FREE_RANGE_COUNT -= 1;
                i = 0;
                continue;
            }

            i += 1;
        }

        if HEAP_FREE_RANGE_COUNT >= HEAP_MAX_FREE_RANGES {
            return false;
        }

        let mut insert_at = 0usize;
        while insert_at < HEAP_FREE_RANGE_COUNT && HEAP_FREE_RANGES[insert_at].base < base {
            insert_at += 1;
        }

        let mut j = HEAP_FREE_RANGE_COUNT;
        while j > insert_at {
            HEAP_FREE_RANGES[j] = HEAP_FREE_RANGES[j - 1];
            j -= 1;
        }

        HEAP_FREE_RANGES[insert_at] = HeapFreeRange { base, size };
        HEAP_FREE_RANGE_COUNT += 1;
    }

    true
}

fn rollback_heap_mapping(base: u32, mapped_pages: usize) {
    for i in 0usize..mapped_pages {
        let va = base + (i * PAGE_SIZE) as u32;
        if let Some(entry) = page_table::get_page(va) {
            let frame = entry & 0xFFFF_F000;
            let _ = page_table::unmap_page(va);
            let _ = physical::free_physical_page(frame);
        }
    }
}

fn map_heap_pages(size: usize) -> Option<*mut u8> {
    let page_count = pages_for(size)?;
    let span = pages_to_bytes(page_count)?;
    let base = allocate_heap_virtual_span(span)?;
    let mut mapped_pages = 0usize;

    for i in 0usize..page_count {
        let frame = match physical::alloc_physical_page() {
            Some(frame) => frame,
            None => {
                rollback_heap_mapping(base, mapped_pages);
                let _ = insert_heap_virtual_span(base, span);
                return None;
            }
        };

        let va = base + (i * PAGE_SIZE) as u32;
        if page_table::map_page(va, frame, PAGE_WRITABLE).is_err() {
            let _ = physical::free_physical_page(frame);
            rollback_heap_mapping(base, mapped_pages);
            let _ = insert_heap_virtual_span(base, span);
            return None;
        }

        mapped_pages += 1;
    }

    Some(base as *mut u8)
}

fn unmap_heap_pages(base: *mut u8, size: usize) -> bool {
    let page_count = match pages_for(size) {
        Some(v) => v,
        None => return false,
    };
    let span = match pages_to_bytes(page_count) {
        Some(v) => v,
        None => return false,
    };

    let base_u32 = base as u32;
    for i in 0usize..page_count {
        let va = base_u32 + (i * PAGE_SIZE) as u32;
        let entry = match page_table::get_page(va) {
            Some(entry) => entry,
            None => return false,
        };

        let frame = entry & 0xFFFF_F000;
        if page_table::unmap_page(va).is_err() {
            return false;
        }
        if !physical::free_physical_page(frame) {
            return false;
        }
    }

    insert_heap_virtual_span(base_u32, span)
}

fn provision_heap_chunk(min_block_total_size: usize) -> bool {
    let chunk_total_size = match chunk_alloc_size(min_block_total_size) {
        Some(size) => size,
        None => {
            pr_warn!(
                "kernel heap chunk size overflow min_block_total_size={}\n",
                min_block_total_size
            );
            return false;
        }
    };

    let chunk = match map_heap_pages(chunk_total_size) {
        Some(ptr) => ptr as *mut HeapChunk,
        None => {
            pr_warn!(
                "kernel heap failed to provision chunk size={}\n",
                chunk_total_size
            );
            return false;
        }
    };

    unsafe {
        (*chunk).magic = HEAP_MAGIC;
        (*chunk).total_size = chunk_total_size;
        (*chunk).prev = ptr::null_mut();
        (*chunk).next = HEAP_CHUNKS;

        if !HEAP_CHUNKS.is_null() {
            (*HEAP_CHUNKS).prev = chunk;
        }
        HEAP_CHUNKS = chunk;

        let first_block = (chunk as *mut u8).add(chunk_header_size()) as *mut BlockHeader;
        let first_block_total_size = chunk_total_size - chunk_header_size();

        if first_block_total_size < minimum_free_block_size() {
            pr_warn!(
                "kernel heap chunk too small chunk_size={} first_block_total={}\n",
                chunk_total_size,
                first_block_total_size
            );
            chunk_list_remove(chunk);
            let _ = unmap_heap_pages(chunk as *mut u8, chunk_total_size);
            return false;
        }

        (*first_block).magic = BLOCK_MAGIC;
        (*first_block).total_size = first_block_total_size;
        (*first_block).usable_size = first_block_total_size - block_alignment_overhead();
        (*first_block).requested_size = 0;
        (*first_block).chunk = chunk;
        (*first_block).prev_free = ptr::null_mut();
        (*first_block).next_free = ptr::null_mut();
        (*first_block).free = 1;
        write_footer(first_block);
        free_list_push(first_block);

        pr_debug!(
            "kernel heap provisioned chunk base={:#x} size={} free_block_total={}\n",
            chunk as usize,
            chunk_total_size,
            first_block_total_size
        );
    }

    true
}

fn block_footer_ptr(block: *mut BlockHeader) -> *mut BlockFooter {
    unsafe { (block as *mut u8).add((*block).total_size - block_footer_size()) as *mut BlockFooter }
}

fn next_block_ptr(block: *mut BlockHeader) -> Option<*mut BlockHeader> {
    unsafe {
        let next_addr = (block as usize).checked_add((*block).total_size)?;
        let chunk = (*block).chunk;
        let chunk_start = chunk as usize;
        let chunk_end = chunk_start.checked_add((*chunk).total_size)?;

        if next_addr >= chunk_end {
            None
        } else {
            Some(next_addr as *mut BlockHeader)
        }
    }
}

fn prev_block_ptr(block: *mut BlockHeader) -> Option<*mut BlockHeader> {
    unsafe {
        let chunk = (*block).chunk;
        let chunk_start = chunk as usize;
        let block_start = block as usize;
        let first_block_start = chunk_start.checked_add(chunk_header_size())?;

        if block_start <= first_block_start {
            return None;
        }

        let footer_addr = block_start.checked_sub(block_footer_size())?;
        if footer_addr < first_block_start {
            return None;
        }

        let footer = footer_addr as *const BlockFooter;
        if (*footer).magic != BLOCK_MAGIC {
            return None;
        }

        let prev_total = (*footer).total_size;
        if prev_total < minimum_free_block_size() {
            return None;
        }

        let prev_start = block_start.checked_sub(prev_total)?;
        if prev_start < first_block_start {
            return None;
        }

        let prev = prev_start as *mut BlockHeader;
        if (*prev).magic != BLOCK_MAGIC || (*prev).total_size != prev_total {
            return None;
        }
        Some(prev)
    }
}

fn write_footer(block: *mut BlockHeader) {
    let footer = block_footer_ptr(block);
    unsafe {
        (*footer).magic = BLOCK_MAGIC;
        (*footer).total_size = (*block).total_size;
    }
}

fn free_list_remove(block: *mut BlockHeader) {
    unsafe {
        let prev = (*block).prev_free;
        let next = (*block).next_free;

        if !prev.is_null() {
            (*prev).next_free = next;
        } else {
            HEAP_FREE_LIST = next;
        }

        if !next.is_null() {
            (*next).prev_free = prev;
        }

        (*block).prev_free = ptr::null_mut();
        (*block).next_free = ptr::null_mut();
    }
}

fn free_list_push(block: *mut BlockHeader) {
    unsafe {
        (*block).prev_free = ptr::null_mut();
        (*block).next_free = HEAP_FREE_LIST;

        if !HEAP_FREE_LIST.is_null() {
            (*HEAP_FREE_LIST).prev_free = block;
        }

        HEAP_FREE_LIST = block;
    }
}

fn chunk_list_remove(chunk: *mut HeapChunk) {
    unsafe {
        let prev = (*chunk).prev;
        let next = (*chunk).next;

        if !prev.is_null() {
            (*prev).next = next;
        } else {
            HEAP_CHUNKS = next;
        }

        if !next.is_null() {
            (*next).prev = prev;
        }

        (*chunk).prev = ptr::null_mut();
        (*chunk).next = ptr::null_mut();
    }
}

fn heap_chunk_from_block(block: *mut BlockHeader) -> Option<*mut HeapChunk> {
    unsafe {
        let chunk = (*block).chunk;
        if chunk.is_null() {
            return None;
        }

        if (*chunk).magic != HEAP_MAGIC {
            return None;
        }

        Some(chunk)
    }
}

fn find_free_block(total_size: usize) -> Option<*mut BlockHeader> {
    unsafe {
        let mut current = HEAP_FREE_LIST;

        while !current.is_null() {
            if (*current).free != 0 && (*current).total_size >= total_size {
                return Some(current);
            }

            current = (*current).next_free;
        }
    }

    None
}

fn split_block(block: *mut BlockHeader, total_size: usize) {
    unsafe {
        let remaining = (*block).total_size - total_size;
        if remaining < minimum_free_block_size() {
            return;
        }

        let remainder = (block as *mut u8).add(total_size) as *mut BlockHeader;
        (*remainder).magic = BLOCK_MAGIC;
        (*remainder).total_size = remaining;
        (*remainder).usable_size = remaining - block_alignment_overhead();
        (*remainder).requested_size = 0;
        (*remainder).chunk = (*block).chunk;
        (*remainder).prev_free = ptr::null_mut();
        (*remainder).next_free = ptr::null_mut();
        (*remainder).free = 1;
        write_footer(remainder);
        free_list_push(remainder);

        (*block).total_size = total_size;
        (*block).usable_size = total_size - block_alignment_overhead();
    }
    write_footer(block);
}

fn release_chunk_if_empty(block: *mut BlockHeader) -> bool {
    let chunk = match heap_chunk_from_block(block) {
        Some(chunk) => chunk,
        None => return false,
    };
    unsafe {
        let first_block = (chunk as *mut u8).add(chunk_header_size()) as *mut BlockHeader;
        let full_free_span = (*chunk).total_size - chunk_header_size();

        if block as usize == first_block as usize && (*block).total_size == full_free_span {
            chunk_list_remove(chunk);
            if !unmap_heap_pages(chunk as *mut u8, (*chunk).total_size) {
                pr_warn!(
                    "kernel heap failed to return chunk pages base={:#x}\n",
                    chunk as usize
                );
            }
            return true;
        }
    }

    false
}

pub fn init_kernel_heap() {
    unsafe {
        HEAP_READY = true;
        HEAP_CHUNKS = ptr::null_mut();
        HEAP_FREE_LIST = ptr::null_mut();
        for i in 0usize..HEAP_MAX_FREE_RANGES {
            HEAP_FREE_RANGES[i] = HeapFreeRange { base: 0, size: 0 };
        }
        HEAP_FREE_RANGES[0] =
            HeapFreeRange { base: HEAP_VM_START, size: HEAP_VM_END - HEAP_VM_START };
        HEAP_FREE_RANGE_COUNT = 1;
    }

    pr_debug!("kernel heap initialized\n");
}

fn ensure_heap_ready() {
    unsafe {
        if !HEAP_READY {
            init_kernel_heap();
        }
    }
}

#[inline(never)]
pub fn kmalloc(size: usize) -> KResult<*mut u8> {
    if size == 0 {
        pr_warn!("kmalloc rejected zero-sized request\n");
        return Err(KernelError::EINVAL);
    }

    ensure_heap_ready();

    let payload_size = align_up(size, HEAP_ALIGNMENT).ok_or_else(|| {
        pr_warn!("kmalloc size overflow size={}\n", size);
        KernelError::ENOMEM
    })?;
    let total_size = align_up(block_alignment_overhead() + payload_size, HEAP_ALIGNMENT)
        .ok_or_else(|| {
            pr_warn!(
                "kmalloc total size overflow size={} payload={}\n",
                size,
                payload_size
            );
            KernelError::ENOMEM
        })?;

    unsafe {
        let mut block = find_free_block(total_size);
        if block.is_none() {
            if !provision_heap_chunk(total_size) {
                pr_debug!(
                    "\x1b\x04mkmalloc\x1bm: no free block found for size={} total_size={}\n",
                    size,
                    total_size
                );
                return Err(KernelError::ENOMEM);
            }
            block = find_free_block(total_size);
        }

        let block = match block {
            Some(block) => block,
            None => {
                pr_warn!(
                    "kmalloc allocator inconsistency size={} total_size={}\n",
                    size,
                    total_size
                );
                return Err(KernelError::EFAULT);
            }
        };
        free_list_remove(block);
        split_block(block, total_size);

        (*block).magic = BLOCK_MAGIC;
        (*block).requested_size = size;
        (*block).usable_size = payload_size;
        (*block).free = 0;
        (*block).prev_free = ptr::null_mut();
        (*block).next_free = ptr::null_mut();
        write_footer(block);

        let user_ptr = (block as *mut u8).add(block_header_size());
        pr_debug!(
            "kmalloc size={} usable={} total={} ptr={:#x}\n",
            size,
            payload_size,
            (*block).total_size,
            user_ptr as usize
        );
        Ok(user_ptr)
    }
}

#[inline(never)]
pub fn kfree(ptr: *mut u8) -> KResult<()> {
    if ptr.is_null() {
        pr_warn!("kfree rejected null pointer\n");
        return Err(KernelError::EINVAL);
    }

    ensure_heap_ready();

    unsafe {
        let block = ptr.sub(block_header_size()) as *mut BlockHeader;
        if (*block).magic != BLOCK_MAGIC {
            pr_warn!("kfree rejected unknown pointer={:#x}\n", ptr as usize);
            return Err(KernelError::EINVAL);
        }

        if (*block).free != 0 {
            pr_warn!("kfree rejected double free ptr={:#x}\n", ptr as usize);
            return Err(KernelError::EINVAL);
        }

        let chunk = match heap_chunk_from_block(block) {
            Some(chunk) => chunk,
            None => {
                pr_warn!("kfree rejected corrupted chunk ptr={:#x}\n", ptr as usize);
                return Err(KernelError::EINVAL);
            }
        };

        (*block).free = 1;
        (*block).requested_size = 0;
        (*block).usable_size = (*block).total_size - block_alignment_overhead();

        let mut merged = block;

        if let Some(next) = next_block_ptr(merged) {
            if (*next).magic == BLOCK_MAGIC && (*next).free != 0 {
                free_list_remove(next);
                (*merged).total_size += (*next).total_size;
                (*merged).usable_size = (*merged).total_size - block_alignment_overhead();
                write_footer(merged);
            }
        }

        if let Some(prev) = prev_block_ptr(merged) {
            if (*prev).magic == BLOCK_MAGIC && (*prev).free != 0 {
                free_list_remove(prev);
                (*prev).total_size += (*merged).total_size;
                (*prev).usable_size = (*prev).total_size - block_alignment_overhead();
                (*prev).free = 1;
                (*prev).requested_size = 0;
                write_footer(prev);
                merged = prev;
            }
        }

        if release_chunk_if_empty(merged) {
            return Ok(());
        }

        (*merged).chunk = chunk;
        free_list_push(merged);
        write_footer(merged);
        pr_debug!(
            "kfree ptr={:#x} total={} merged_block={:#x}\n",
            ptr as usize,
            (*merged).total_size,
            merged as usize
        );
        Ok(())
    }
}

#[inline(never)]
pub fn ksize(ptr: *const u8) -> KResult<usize> {
    if ptr.is_null() {
        return Err(KernelError::EINVAL);
    }

    ensure_heap_ready();

    unsafe {
        let block = ptr.sub(block_header_size()) as *const BlockHeader;
        if (*block).magic != BLOCK_MAGIC || (*block).free != 0 {
            return Err(KernelError::EFAULT);
        }

        let chunk = (*block).chunk;
        if chunk.is_null() || (*chunk).magic != HEAP_MAGIC {
            return Err(KernelError::EFAULT);
        }

        Ok((*block).requested_size)
    }
}

pub fn debug_stats() -> HeapStats {
    unsafe {
        let ready = HEAP_READY;
        if !ready {
            return HeapStats {
                ready,
                chunk_count: 0,
                chunk_bytes: 0,
                free_block_count: 0,
                free_bytes: 0,
                used_block_count: 0,
                used_requested_bytes: 0,
                used_usable_bytes: 0,
            };
        }

        let mut chunk_count = 0usize;
        let mut chunk_bytes = 0usize;
        let mut free_block_count = 0usize;
        let mut free_bytes = 0usize;
        let mut used_block_count = 0usize;
        let mut used_requested_bytes = 0usize;
        let mut used_usable_bytes = 0usize;

        let mut chunk = HEAP_CHUNKS;
        while !chunk.is_null() {
            if (*chunk).magic == HEAP_MAGIC {
                chunk_count = chunk_count.saturating_add(1);
                chunk_bytes = chunk_bytes.saturating_add((*chunk).total_size);

                let mut block = (chunk as *mut u8).add(chunk_header_size()) as *mut BlockHeader;
                let chunk_end = (chunk as usize).saturating_add((*chunk).total_size);

                while (block as usize) < chunk_end {
                    if (*block).magic != BLOCK_MAGIC || (*block).total_size == 0 {
                        break;
                    }

                    if (*block).free != 0 {
                        free_block_count = free_block_count.saturating_add(1);
                        free_bytes = free_bytes.saturating_add((*block).usable_size);
                    } else {
                        used_block_count = used_block_count.saturating_add(1);
                        used_requested_bytes =
                            used_requested_bytes.saturating_add((*block).requested_size);
                        used_usable_bytes = used_usable_bytes.saturating_add((*block).usable_size);
                    }

                    let next = (block as usize).saturating_add((*block).total_size);
                    if next <= block as usize {
                        break;
                    }
                    block = next as *mut BlockHeader;
                }
            }

            chunk = (*chunk).next;
        }

        HeapStats {
            ready,
            chunk_count,
            chunk_bytes,
            free_block_count,
            free_bytes,
            used_block_count,
            used_requested_bytes,
            used_usable_bytes,
        }
    }
}
