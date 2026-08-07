use core::{fmt, ptr};

use crate::{
    error::{KResult, KernelError},
    paging,
};

pub fn print_memstat(mut emit: impl FnMut(&fmt::Arguments<'_>)) {
    let total_pages = paging::total_physical_pages();
    let free_pages = paging::free_physical_pages();
    let used_pages = total_pages.saturating_sub(free_pages);
    let page_size = paging::physical_page_size();
    let total_phys_kib = total_pages.saturating_mul(page_size) / 1024;
    let free_phys_kib = free_pages.saturating_mul(page_size) / 1024;

    let vstats = paging::vmem_debug_stats();
    let hstats = paging::kernel_heap_debug_stats();

    emit(&format_args!(
        "physical: total_pages={} free_pages={} used_pages={} total_kib={} free_kib={}\n",
        total_pages, free_pages, used_pages, total_phys_kib, free_phys_kib
    ));
    emit(&format_args!(
        "vmem: range=[{:#010x}, {:#010x}) total_kib={} free_kib={} allocs={} alloc_bytes={} free_ranges={}\n",
        vstats.range_start,
        vstats.range_end,
        (vstats.total_bytes as usize) / 1024,
        (vstats.free_bytes as usize) / 1024,
        vstats.alloc_count,
        vstats.alloc_bytes,
        vstats.free_ranges
    ));
    emit(&format_args!(
        "kheap: ready={} chunks={} chunk_bytes={} free_blocks={} free_bytes={} used_blocks={} used_req_bytes={} used_usable_bytes={}\n",
        hstats.ready,
        hstats.chunk_count,
        hstats.chunk_bytes,
        hstats.free_block_count,
        hstats.free_bytes,
        hstats.used_block_count,
        hstats.used_requested_bytes,
        hstats.used_usable_bytes
    ));
}

pub fn print_memdebug(mut emit: impl FnMut(&fmt::Arguments<'_>)) {
    print_memstat(&mut emit);

    emit(&format_args!("vmem active allocations:\n"));
    let mut alloc_count = 0usize;
    paging::vmem_debug_for_each_alloc(|base, size, pages| {
        alloc_count += 1;
        emit(&format_args!(
            "  alloc#{:02} base={:#010x} size={} pages={}\n",
            alloc_count, base, size, pages
        ));
    });
    if alloc_count == 0 {
        emit(&format_args!("  (none)\n"));
    }

    emit(&format_args!("vmem free ranges:\n"));
    let mut free_count = 0usize;
    paging::vmem_debug_for_each_free_range(|base, size| {
        free_count += 1;
        let end = base.saturating_add(size);
        emit(&format_args!(
            "  range#{:02} [{:#010x}, {:#010x}) size={}\n",
            free_count, base, end, size
        ));
    });
    if free_count == 0 {
        emit(&format_args!("  (none)\n"));
    }
}

pub fn debug_page_entry(addr: u32, mut emit: impl FnMut(&fmt::Arguments<'_>)) {
    let page_base = addr & 0xFFFF_F000;
    match paging::get_page(page_base) {
        Some(entry) => {
            let phys = entry & 0xFFFF_F000;
            let flags = entry & 0x0000_0FFF;
            emit(&format_args!(
                "pte: va={:#010x} page={:#010x} entry={:#010x} pa={:#010x} flags={:#05x}\n",
                addr, page_base, entry, phys, flags
            ));
            emit(&format_args!(
                "  present={} writable={} user={} huge={}\n",
                (entry & paging::PAGE_PRESENT) != 0,
                (entry & paging::PAGE_WRITABLE) != 0,
                (entry & paging::PAGE_USER) != 0,
                (entry & paging::PAGE_PAGE_SIZE_4MB) != 0
            ));
        }
        None => {
            emit(&format_args!(
                "pte: va={:#010x} page={:#010x} not mapped\n",
                addr, page_base
            ));
        }
    }
}

pub fn dump_virtual_memory(start_addr: u32, len: usize, mut emit: impl FnMut(&fmt::Arguments<'_>)) {
    let end_addr = match (start_addr as usize).checked_add(len) {
        Some(v) if v <= u32::MAX as usize => v as u32,
        _ => {
            emit(&format_args!("address range overflow\n"));
            return;
        }
    };

    emit(&format_args!(
        "memdump: [{:#010x}, {:#010x}) len={}\n",
        start_addr, end_addr, len
    ));

    let mut offset = 0usize;
    while offset < len {
        let line_addr = start_addr.wrapping_add(offset as u32);
        emit(&format_args!("{:#010x}: ", line_addr));

        let mut ascii = [b'.'; 16];
        for i in 0usize..16 {
            let pos = offset + i;
            if pos >= len {
                emit(&format_args!("   "));
                continue;
            }

            let byte_addr = start_addr.wrapping_add(pos as u32);
            let page_base = byte_addr & 0xFFFF_F000;
            if paging::get_page(page_base).is_none() {
                emit(&format_args!("?? "));
                ascii[i] = b'?';
                continue;
            }

            let value = unsafe { ptr::read_volatile(byte_addr as *const u8) };
            emit(&format_args!("{:02x} ", value));
            ascii[i] = if value.is_ascii_graphic() || value == b' ' {
                value
            } else {
                b'.'
            };
        }

        emit(&format_args!(" |"));
        for i in 0usize..16 {
            let pos = offset + i;
            if pos >= len {
                break;
            }
            emit(&format_args!("{}", ascii[i] as char));
        }
        emit(&format_args!("|\n"));

        offset = offset.saturating_add(16);
    }
}

#[allow(dead_code)]
pub fn dump_physical_memory(start_addr: u32, len: usize, mut emit: impl FnMut(&fmt::Arguments<'_>)) {
    let end_addr = match (start_addr as usize).checked_add(len) {
        Some(v) if v <= u32::MAX as usize => v as u32,
        _ => {
            emit(&format_args!("address range overflow\n"));
            return;
        }
    };

    emit(&format_args!(
        "memdump: [{:#010x}, {:#010x}) len={}\n",
        start_addr, end_addr, len
    ));

    let mut offset = 0usize;
    while offset < len {
        let line_addr = start_addr.wrapping_add(offset as u32);
        emit(&format_args!("{:#010x}: ", line_addr));

        let mut ascii = [b'.'; 16];
        for i in 0usize..16 {
            let pos = offset + i;
            if pos >= len {
                emit(&format_args!("   "));
                continue;
            }

            let byte_addr = start_addr.wrapping_add(pos as u32);
            let value = unsafe { ptr::read_volatile(byte_addr as *const u8) };
            emit(&format_args!("{:02x} ", value));
            ascii[i] = if value.is_ascii_graphic() || value == b' ' {
                value
            } else {
                b'.'
            };
        }

        emit(&format_args!(" |"));
        for i in 0usize..16 {
            let pos = offset + i;
            if pos >= len {
                break;
            }
            emit(&format_args!("{}", ascii[i] as char));
        }
        emit(&format_args!("|\n"));

        offset = offset.saturating_add(16);
    }
}

pub fn run_memtest(features: &str, mut emit: impl FnMut(&fmt::Arguments<'_>)) {
    let mut run_physical = false;
    let mut run_vmem = false;
    let mut run_heap = false;
    let mut run_page = false;

    if features.is_empty() || features == "all" {
        run_physical = true;
        run_vmem = true;
        run_heap = true;
        run_page = true;
    } else {
        for token in features.split(|c: char| c == ',' || c.is_ascii_whitespace()) {
            if token.is_empty() {
                continue;
            }

            match token {
                "physical" => run_physical = true,
                "vmem" => run_vmem = true,
                "heap" => run_heap = true,
                "page" => run_page = true,
                "all" => {
                    run_physical = true;
                    run_vmem = true;
                    run_heap = true;
                    run_page = true;
                }
                _ => {
                    emit(&format_args!(
                        "usage: memtest [physical,vmem,heap,page,all]\n"
                    ));
                    emit(&format_args!("unknown feature: {}\n", token));
                    return;
                }
            }
        }
    }

    emit(&format_args!("memtest: running selected tests\n"));

    let mut total = 0usize;
    let mut passed = 0usize;

    if run_physical {
        total += 1;
        if memtest_physical_roundtrip() {
            passed += 1;
            emit(&format_args!("  [PASS] physical\n"));
        } else {
            emit(&format_args!("  [FAIL] physical\n"));
        }
    }

    if run_vmem {
        total += 1;
        if memtest_vmem_roundtrip() {
            passed += 1;
            emit(&format_args!("  [PASS] vmem\n"));
        } else {
            emit(&format_args!("  [FAIL] vmem\n"));
        }
    }

    if run_heap {
        total += 1;
        match memtest_heap_roundtrip() {
            Ok(()) => {
                passed += 1;
                emit(&format_args!("  [PASS] heap\n"));
            }
            Err(e) => {
                emit(&format_args!("  [FAIL] heap ({:?})\n", e));
            }
        }
    }

    if run_page {
        total += 1;
        if memtest_page_roundtrip() {
            passed += 1;
            emit(&format_args!("  [PASS] page\n"));
        } else {
            emit(&format_args!("  [FAIL] page\n"));
        }
    }

    emit(&format_args!(
        "memtest summary: passed={}/{} failed={}\n",
        passed,
        total,
        total.saturating_sub(passed)
    ));
}

fn memtest_physical_roundtrip() -> bool {
    let free_before = paging::free_physical_pages();
    let Ok(frame) = paging::alloc_physical_page() else {
        return false;
    };

    let free_after_alloc = paging::free_physical_pages();
    if free_after_alloc.saturating_add(1) != free_before {
        let _ = paging::free_physical_page(frame);
        return false;
    }

    if paging::free_physical_page(frame).is_err() {
        return false;
    }

    paging::free_physical_pages() == free_before
}

fn memtest_vmem_roundtrip() -> bool {
    let Some(ptr) = paging::vmalloc(4096).ok() else {
        return false;
    };

    if paging::vsize(ptr as *const u8).is_ok_and(|size| size != 4096) {
        let _ = paging::vfree(ptr);
        return false;
    }

    paging::vfree(ptr).is_ok()
}

fn memtest_heap_roundtrip() -> KResult<()> {
    let ptr = paging::kmalloc(128)?;

    paging::ksize(ptr as *const u8)
        .is_ok_and(|size| size == 128)
        .then_some(())
        .ok_or(KernelError::EINVAL)?;
    paging::kfree(ptr)
}

fn memtest_page_roundtrip() -> bool {
    const TEST_USER_VA: u32 = 0x0800_0000;

    let Ok(frame) = paging::alloc_physical_page() else {
        return false;
    };

    let map_ok = paging::map_page(
        TEST_USER_VA,
        frame,
        paging::PAGE_WRITABLE | paging::PAGE_USER,
    )
    .is_ok();
    if !map_ok {
        let _ = paging::free_physical_page(frame);
        return false;
    }

    let mapped =
        matches!(paging::get_page(TEST_USER_VA), Some(entry) if (entry & 0xFFFF_F000) == frame);

    let _ = paging::unmap_page(TEST_USER_VA);
    let _ = paging::free_physical_page(frame);

    mapped
}
