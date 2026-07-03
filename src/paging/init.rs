use super::frame_allocator;
use super::kernel_heap;
use super::multiboot::{
    multiboot_info_from_addr, set_boot_multiboot_info_addr, MULTIBOOT_BOOTLOADER_MAGIC,
    MULTIBOOT_INFO_HAS_BASIC_MEMORY,
};
use super::page_table;
use super::physical;
use super::vmem;
use super::{PAGE_USER, PAGE_WRITABLE};
use crate::x86;
use crate::{pr_debug, pr_err, pr_warn};

pub const PAGE_SIZE: usize = 4096;
pub const USER_SPACE_START: usize = 0x0000_1000;
pub const KERNEL_SPACE_START: usize = 0xC000_0000;

fn test_start(name: &str) {
    pr_debug!(
        "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] \x1b\x01mSTART\x1bm\n",
        name
    );
}

fn test_pass(name: &str) {
    pr_debug!(
        "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] \x1b\x02mPASS\x1bm\n",
        name
    );
}

fn test_fail(name: &str, reason: &str) {
    pr_debug!(
        "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] \x1b\x04mFAIL: {}\x1bm\n",
        name,
        reason
    );
    pr_err!("[paging-selftest] test {} failed: {}\n", name, reason);
}

fn test_identity_page_present() -> bool {
    const NAME: &str = "identity-page";
    test_start(NAME);

    match page_table::get_page_bootstrap(0x0000_0000) {
        Some(entry) => {
            pr_debug!(
                "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] entry={:#x}\n",
                NAME,
                entry
            );
            test_pass(NAME);
            true
        }
        None => {
            test_fail(NAME, "identity page lookup returned none at 0x0");
            false
        }
    }
}

fn test_bootstrap_map_get_roundtrip() -> bool {
    const NAME: &str = "bootstrap-map-get";
    let probe_addr = 0x003F_F000;
    test_start(NAME);

    match page_table::map_page_bootstrap(probe_addr, probe_addr, PAGE_WRITABLE) {
        Ok(()) => match page_table::get_page_bootstrap(probe_addr) {
            Some(entry) => {
                let phys = entry & 0xFFFF_F000;
                if phys == probe_addr {
                    pr_debug!(
                        "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] va={:#x} -> pa={:#x}\n",
                        NAME,
                        probe_addr,
                        phys
                    );
                    test_pass(NAME);
                    true
                } else {
                    pr_err!(
                        "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] mismatch va={:#x} expected_pa={:#x} got_pa={:#x}\n",
                        NAME,
                        probe_addr,
                        probe_addr,
                        phys
                    );
                    false
                }
            }
            None => {
                test_fail(NAME, "mapped page not readable after map_page_bootstrap");
                false
            }
        },
        Err(e) => {
            pr_err!(
                "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] map failed: {:?}\n",
                NAME,
                e
            );
            false
        }
    }
}

fn test_bootstrap_guard_rejects_unsupported_pde() -> bool {
    const NAME: &str = "bootstrap-guard";
    test_start(NAME);

    match page_table::map_page_bootstrap(0x0080_0000, 0x0080_0000, PAGE_WRITABLE) {
        Ok(()) => {
            test_fail(NAME, "expected unsupported-PDE mapping to fail");
            false
        }
        Err(_) => {
            test_pass(NAME);
            true
        }
    }
}

fn test_physical_alloc_free_roundtrip() -> bool {
    const NAME: &str = "physical-alloc-free";
    test_start(NAME);

    let free_before = physical::free_physical_pages();
    let Ok(frame) = physical::alloc_physical_page() else {
        test_fail(NAME, "alloc_physical_page returned none");
        return false;
    };

    let free_after_alloc = physical::free_physical_pages();
    if free_after_alloc + 1 != free_before {
        pr_err!(
            "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] free-count mismatch after alloc before={} after={}\n",
            NAME,
            free_before,
            free_after_alloc
        );
        return false;
    }

    if physical::free_physical_page(frame).is_err() {
        pr_err!(
            "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] free_physical_page failed frame={:#x}\n",
            NAME,
            frame
        );
        return false;
    }

    let free_after_free = physical::free_physical_pages();
    if free_after_free != free_before {
        pr_err!(
            "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] free-count mismatch after free expected={} got={}\n",
            NAME,
            free_before,
            free_after_free
        );
        return false;
    }

    test_pass(NAME);
    true
}

fn test_vmalloc_vfree_vsize() -> bool {
    const NAME: &str = "vmalloc-vfree-vsize";
    test_start(NAME);

    let Ok(ptr) = vmem::vmalloc(6000) else {
        test_fail(NAME, "vmalloc returned none");
        return false;
    };

    let mut ok = true;
    match vmem::vsize(ptr as *const u8) {
        Ok(6000) => {}
        Ok(other) => {
            pr_err!(
                "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] size mismatch ptr={:#x} got={} expected=6000\n",
                NAME,
                ptr as usize,
                other
            );
            ok = false;
        }
        Err(_) => {
            pr_err!(
                "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] vsize returned none ptr={:#x}\n",
                NAME,
                ptr as usize
            );
            ok = false;
        }
    }

    match vmem::vfree(ptr) {
        Ok(()) => {}
        Err(e) => {
            pr_err!(
                "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] vfree failed ptr={:#x} e={:?}\n",
                NAME,
                ptr as usize,
                e
            );
            ok = false;
        }
    }

    if ok {
        test_pass(NAME);
    }
    ok
}

fn test_kmalloc_kfree_ksize() -> bool {
    const NAME: &str = "kmalloc-kfree-ksize";
    test_start(NAME);

    let Ok(ptr) = kernel_heap::kmalloc(128) else {
        test_fail(NAME, "kmalloc returned none");
        return false;
    };

    let mut ok = true;
    match kernel_heap::ksize(ptr as *const u8) {
        Ok(128) => {}
        Ok(other) => {
            pr_err!(
                "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] size mismatch ptr={:#x} got={} expected=128\n",
                NAME,
                ptr as usize,
                other
            );
            ok = false;
        }
        Err(e) => {
            pr_err!(
                "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] ksize returned error ptr={:#x} error={:?}\n",
                NAME,
                ptr as usize,
                e
            );
            ok = false;
        }
    }

    match kernel_heap::kfree(ptr) {
        Ok(()) => {}
        Err(e) => {
            pr_err!(
                "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] kfree failed ptr={:#x} e={:?}\n",
                NAME,
                ptr as usize,
                e
            );
            ok = false;
        }
    }

    if ok {
        test_pass(NAME);
    }
    ok
}

fn test_kmalloc_reuse_after_free() -> bool {
    const NAME: &str = "kmalloc-reuse-after-free";
    test_start(NAME);

    let Ok(first) = kernel_heap::kmalloc(128) else {
        test_fail(NAME, "first kmalloc returned none");
        return false;
    };

    let Ok(second) = kernel_heap::kmalloc(96) else {
        let _ = kernel_heap::kfree(first);
        test_fail(NAME, "second kmalloc returned none");
        return false;
    };

    let mut ok = true;
    if first == second {
        pr_err!(
            "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] distinct allocations aliased ptr={:#x}\n",
            NAME,
            first as usize
        );
        ok = false;
    }

    match kernel_heap::ksize(first as *const u8) {
        Ok(128) => {}
        Ok(other) => {
            pr_err!(
                "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] size mismatch ptr={:#x} got={} expected=128\n",
                NAME,
                first as usize,
                other
            );
            ok = false;
        }
        Err(e) => {
            pr_err!(
                "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] ksize returned error ptr={:#x} error={:?}\n",
                NAME,
                first as usize,
                e
            );
            ok = false;
        }
    }

    match kernel_heap::ksize(second as *const u8) {
        Ok(96) => {}
        Ok(other) => {
            pr_err!(
                "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] size mismatch ptr={:#x} got={} expected=96\n",
                NAME,
                second as usize,
                other
            );
            ok = false;
        }
        Err(e) => {
            pr_err!(
                "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] ksize returned error ptr={:#x} error={:?}\n",
                NAME,
                second as usize,
                e
            );
            ok = false;
        }
    }

    match kernel_heap::kfree(first) {
        Ok(()) => {}
        Err(e) => {
            pr_err!(
                "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] kfree failed ptr={:#x} e={:?}\n",
                NAME,
                first as usize,
                e
            );
            ok = false;
        }
    }

    let Ok(reused) = kernel_heap::kmalloc(80) else {
        test_fail(NAME, "reuse kmalloc returned none");
        let _ = kernel_heap::kfree(second);
        return false;
    };

    if reused != first {
        pr_err!(
            "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] allocator did not reuse freed block reused={:#x} first={:#x}\n",
            NAME,
            reused as usize,
            first as usize
        );
        ok = false;
    }

    if kernel_heap::ksize(reused as *const u8) != Ok(80) {
        pr_err!(
            "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] reused block size mismatch ptr={:#x}\n",
            NAME,
            reused as usize
        );
        ok = false;
    }

    match kernel_heap::kfree(second) {
        Ok(()) => {}
        Err(e) => {
            pr_err!(
                "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] kfree failed ptr={:#x} e={:?}\n",
                NAME,
                second as usize,
                e
            );
            ok = false;
        }
    }

    match kernel_heap::kfree(reused) {
        Ok(()) => {}
        Err(e) => {
            pr_err!(
                "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] kfree failed ptr={:#x} e={:?}\n",
                NAME,
                reused as usize,
                e
            );
            ok = false;
        }
    }

    if ok {
        test_pass(NAME);
    }
    ok
}

fn test_kernel_user_rights_guard() -> bool {
    const NAME: &str = "kernel-user-rights";
    test_start(NAME);

    match page_table::map_page(KERNEL_SPACE_START as u32, 0x0010_0000, PAGE_USER) {
        Ok(()) => {
            test_fail(NAME, "kernel mapping accepted PAGE_USER unexpectedly");
            false
        }
        Err(_) => {
            test_pass(NAME);
            true
        }
    }
}

fn test_user_map_get_unmap_roundtrip() -> bool {
    const NAME: &str = "user-map-get-unmap";
    test_start(NAME);

    let Ok(frame) = physical::alloc_physical_page() else {
        test_fail(NAME, "no physical frame available for user mapping test");
        return false;
    };

    let user_va = (USER_SPACE_START + PAGE_SIZE) as u32;
    let mut ok = true;

    match page_table::map_page(user_va, frame, PAGE_WRITABLE | PAGE_USER) {
        Ok(()) => match page_table::get_page(user_va) {
            Some(entry) => {
                let got = entry & 0xFFFF_F000;
                if got != frame {
                    pr_err!(
                        "[paging-selftest:{}] map/get mismatch va={:#x} expected={:#x} got={:#x}\n",
                        NAME,
                        user_va,
                        frame,
                        got
                    );
                    ok = false;
                }
            }
            None => {
                test_fail(NAME, "user page lookup failed after map");
                ok = false;
            }
        },
        Err(e) => {
            pr_err!(
                "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] user map failed: {:?}\n",
                NAME,
                e
            );
            ok = false;
        }
    }

    match page_table::unmap_page(user_va) {
        Ok(()) => {}
        Err(e) => {
            pr_err!(
                "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] user page unmap failed va={:#x} e={:?}\n",
                NAME,
                user_va,
                e
            );
            ok = false;
        }
    }

    let _ = physical::free_physical_page(frame);

    if ok {
        test_pass(NAME);
    }
    ok
}

fn test_virtual_reuse_after_free() -> bool {
    const NAME: &str = "virtual-reuse";
    test_start(NAME);

    let first = vmem::vmalloc(4096);
    let second = first.and_then(|ptr| {
        vmem::vfree(ptr)?;
        vmem::vmalloc(4096)
    });

    let result = match (first, second) {
        (Ok(first_ptr), Ok(second_ptr)) if first_ptr == second_ptr => {
            pr_debug!(
                "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] reused ptr={:#x}\n",
                NAME,
                first_ptr as usize
            );
            test_pass(NAME);
            true
        }
        (Ok(first_ptr), Ok(second_ptr)) => {
            pr_warn!(
                "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] returned different ptrs first={:#x} second={:#x}\n",
                NAME,
                first_ptr as usize,
                second_ptr as usize
            );
            true
        }
        (Ok(_), Err(_)) => {
            test_fail(NAME, "second allocation failed after successful free");
            false
        }
        (Err(_), _) => {
            test_fail(NAME, "initial allocation failed");
            false
        }
    };

    if second.is_ok() {
        let _ = second.and_then(|ptr| {
            vmem::vfree(ptr).or_else(|e| {
                pr_warn!(
                    "[paging-selftest:\x1b\x1f;\x00m{}\x1bm] vfree failed ptr={:#x} e={:?}\n",
                    NAME,
                    ptr as usize,
                    e
                );
                Ok(())
            })
        });
    }

    result
}

fn run_bootstrap_self_tests() {
    pr_debug!("[paging-selftest] starting bootstrap test suite\n");

    struct TestCase {
        name: &'static str,
        passed: bool,
        test_fn: fn() -> bool,
    }

    let mut tests = [
        TestCase {
            name: "identity-page",
            passed: false,
            test_fn: test_identity_page_present,
        },
        TestCase {
            name: "bootstrap-map-get",
            passed: false,
            test_fn: test_bootstrap_map_get_roundtrip,
        },
        TestCase {
            name: "bootstrap-guard",
            passed: false,
            test_fn: test_bootstrap_guard_rejects_unsupported_pde,
        },
        TestCase {
            name: "physical-alloc-free",
            passed: false,
            test_fn: test_physical_alloc_free_roundtrip,
        },
        TestCase {
            name: "vmalloc-vfree-vsize",
            passed: false,
            test_fn: test_vmalloc_vfree_vsize,
        },
        TestCase {
            name: "kmalloc-kfree-ksize",
            passed: false,
            test_fn: test_kmalloc_kfree_ksize,
        },
        TestCase {
            name: "kmalloc-reuse-after-free",
            passed: false,
            test_fn: test_kmalloc_reuse_after_free,
        },
        TestCase {
            name: "kernel-user-rights",
            passed: false,
            test_fn: test_kernel_user_rights_guard,
        },
        TestCase {
            name: "user-map-get-unmap",
            passed: false,
            test_fn: test_user_map_get_unmap_roundtrip,
        },
        TestCase {
            name: "virtual-reuse",
            passed: false,
            test_fn: test_virtual_reuse_after_free,
        },
    ];

    for test in tests.iter_mut() {
        if (test.test_fn)() {
            test.passed = true;
        }
    }

    for test in &tests {
        if !test.passed {
            pr_err!("[paging-selftest] test {} failed\n", test.name);
        }
    }

    pr_debug!("[paging-selftest] bootstrap test \x1b\x0a;\x14msuite\x1bm complete\n");
}

pub fn init_paging(multiboot_magic: u32, multiboot_info_addr: u32) {
    assert_eq!(
        multiboot_magic, MULTIBOOT_BOOTLOADER_MAGIC,
        "Invalid multiboot magic: {:#x}",
        multiboot_magic
    );

    let mb_info = multiboot_info_from_addr(multiboot_info_addr);
    set_boot_multiboot_info_addr(multiboot_info_addr);

    if (mb_info.flags & MULTIBOOT_INFO_HAS_BASIC_MEMORY) != 0 {
        pr_debug!(
            "Multiboot low/high memory: {} KiB / {} KiB\n",
            mb_info.mem_lower,
            mb_info.mem_upper
        );
    }

    frame_allocator::init_from_multiboot(mb_info);
    vmem::init_vmem();
    kernel_heap::init_kernel_heap();

    pr_debug!(
        "Physical frames: total={} free={}\n",
        frame_allocator::total_frame_count(),
        frame_allocator::free_frame_count()
    );

    page_table::enable_bootstrap_paging();
    pr_debug!(
        "Paging enabled: cr3={:#x} bootstrap_pd={:#x}\n",
        x86::read_cr3(),
        page_table::bootstrap_directory_phys_addr()
    );

    page_table::mark_paging_initialized();
    run_bootstrap_self_tests();
}
