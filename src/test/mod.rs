use crate::fs::{self, VfsNodeType};
use crate::paging;
use crate::pr_info;

#[unsafe(link_section = ".init.text")]
pub(crate) fn fs_boot_probe() {
    unsafe {
        let root = fs::ROOT_NODE;
        if root.is_null() {
            pr_info!("fs_boot_probe: root not ready\n");
            return;
        }

        let dev_dir = match fs::resolve_path("/dev", root) {
            Ok(node) => node,
            Err(err) => {
                pr_info!("fs_boot_probe: /dev unavailable: {:?}\n", err);
                return;
            }
        };

        let mnt_dir = match fs::resolve_path("/mnt", root) {
            Ok(node) => node,
            Err(err) => {
                pr_info!("fs_boot_probe: /mnt unavailable: {:?}\n", err);
                return;
            }
        };

        if let Err(err) = fs::mount_node(mnt_dir, dev_dir) {
            pr_info!("fs_boot_probe: mount helper failed: {:?}\n", err);
            return;
        }

        if let Err(err) = fs::umount_node(mnt_dir) {
            pr_info!("fs_boot_probe: umount helper failed: {:?}\n", err);
            return;
        }

        pr_info!("fs_boot_probe: mount helpers validated\n");

        // Verify procfs resolution and reading
        if let Ok(version_node) = fs::resolve_path("/proc/version", root) {
            let mut vbuf = [0u8; 64];
            if (*version_node).read(&mut vbuf, 0).is_ok() {
                pr_info!("fs_boot_probe: procfs pseudo-filesystem validated\n");
            }
        }
    }
}

crate::late_initcall!(fs_boot_probe);

#[unsafe(link_section = ".init.text")]
pub(crate) fn vmem_boot_probe() {
    // 1. Basic allocation, sizing, write, guard page check, and free
    let ptr = match paging::vmalloc(6000) {
        Ok(p) => p,
        Err(e) => {
            pr_info!("vmem_boot_probe: vmalloc(6000) failed: {:?}\n", e);
            return;
        }
    };

    if let Ok(size) = paging::vsize(ptr as *const u8) {
        if size != 6000 {
            pr_info!(
                "vmem_boot_probe: vsize mismatch: got {} expected 6000\n",
                size
            );
            let _ = paging::vfree(ptr);
            return;
        }
    } else {
        pr_info!("vmem_boot_probe: vsize failed\n");
        let _ = paging::vfree(ptr);
        return;
    }

    // Write test pattern
    unsafe {
        core::ptr::write_bytes(ptr, 0x5A, 6000);
        let mut ok = true;
        for i in 0..6000 {
            if *ptr.add(i) != 0x5A {
                ok = false;
                break;
            }
        }
        if !ok {
            pr_info!("vmem_boot_probe: memory verification pattern failed\n");
            let _ = paging::vfree(ptr);
            return;
        }
    }

    // Guard page check: page_count is 2 (8192 bytes). Guard page is at ptr + 8192.
    // It must NOT be mapped in the page table.
    let guard_va = (ptr as u32) + 8192;
    if paging::get_page(guard_va).is_some() {
        pr_info!(
            "vmem_boot_probe: guard page at {:#x} is mapped (expected unmapped)\n",
            guard_va
        );
        let _ = paging::vfree(ptr);
        return;
    }

    if let Err(e) = paging::vfree(ptr) {
        pr_info!("vmem_boot_probe: vfree failed: {:?}\n", e);
        return;
    }

    // Verify unmapped after free
    if paging::get_page(ptr as u32).is_some() {
        pr_info!("vmem_boot_probe: freed page still mapped\n");
        return;
    }

    // 2. Virtual address reuse test
    let first = paging::vmalloc(4096);
    let second = first.and_then(|p1| {
        let _ = paging::vfree(p1);
        paging::vmalloc(4096)
    });
    match (first, second) {
        (Ok(p1), Ok(p2)) => {
            if p1 == p2 {
                pr_info!(
                    "vmem_boot_probe: virtual reuse confirmed (ptr={:#x})\n",
                    p1 as usize
                );
            }
            let _ = paging::vfree(p2);
        }
        _ => {
            pr_info!("vmem_boot_probe: virtual reuse test failed\n");
            return;
        }
    }

    // 3. ioremap / iounmap test
    let io_ptr = match paging::ioremap(0xFEE0_0000, 4096) {
        Ok(p) => p,
        Err(e) => {
            pr_info!("vmem_boot_probe: ioremap failed: {:?}\n", e);
            return;
        }
    };
    if let Ok(sz) = paging::vsize(io_ptr as *const u8) {
        if sz != 4096 {
            pr_info!("vmem_boot_probe: ioremap size mismatch got {}\n", sz);
        }
    }
    if let Err(e) = paging::iounmap(io_ptr) {
        pr_info!("vmem_boot_probe: iounmap failed: {:?}\n", e);
        return;
    }

    // 4. Fragmentation and coalescence test
    let mut ptrs = [core::ptr::null_mut::<u8>(); 10];
    let mut alloc_ok = true;
    for i in 0..10 {
        match paging::vmalloc((i + 1) * 4096) {
            Ok(p) => ptrs[i] = p,
            Err(_) => {
                alloc_ok = false;
                break;
            }
        }
    }
    if !alloc_ok {
        pr_info!("vmem_boot_probe: batch vmalloc failed\n");
        for p in ptrs {
            if !p.is_null() {
                let _ = paging::vfree(p);
            }
        }
        return;
    }

    // Free odd indices (creating alternating holes)
    for i in (1..10).step_by(2) {
        let _ = paging::vfree(ptrs[i]);
        ptrs[i] = core::ptr::null_mut();
    }

    // Free remaining even indices
    for i in (0..10).step_by(2) {
        let _ = paging::vfree(ptrs[i]);
        ptrs[i] = core::ptr::null_mut();
    }

    // Verify complete coalescence back to 1 free range
    let stats = paging::vmem_debug_stats();
    if stats.free_ranges != 1 || stats.alloc_count != 0 {
        pr_info!(
            "vmem_boot_probe: coalescence check failed free_ranges={} alloc_count={}\n",
            stats.free_ranges,
            stats.alloc_count
        );
        return;
    }

    pr_info!("vmem_boot_probe: bitmap allocator, guard pages & ioremap validated\n");
}

crate::late_initcall!(vmem_boot_probe);

#[unsafe(link_section = ".init.text")]
pub(crate) fn kernel_features_boot_probe() {
    unsafe {
        // 1. Test VFS node reference counting
        if let Ok(test_node) = fs::alloc_vfs_node() {
            if (*test_node).ref_count != 0 {
                pr_info!("features_boot_probe: initial VFS refcount != 0\n");
                return;
            }
            fs::vfs_ref_get(test_node);
            if (*test_node).ref_count != 1 {
                pr_info!("features_boot_probe: VFS vfs_ref_get failed\n");
                return;
            }
            fs::vfs_ref_put(test_node);
            if (*test_node).ref_count != 0 {
                pr_info!("features_boot_probe: VFS vfs_ref_put failed\n");
                return;
            }
            pr_info!("features_boot_probe: VFS refcounting validated\n");
        }

        // 2. Test Pipe buffer read/write
        if let Ok(pipe_id) = crate::pipe::create_pipe() {
            let msg = b"KFS_PIPE_TEST";
            let mut read_buf = [0u8; 16];
            let write_res = crate::pipe::pipe_write(pipe_id, msg);
            if write_res != Ok(msg.len()) {
                pr_info!("features_boot_probe: pipe_write failed: {:?}\n", write_res);
                return;
            }
            let read_res = crate::pipe::pipe_read(pipe_id, &mut read_buf[..msg.len()]);
            if read_res != Ok(msg.len()) || &read_buf[..msg.len()] != msg {
                pr_info!("features_boot_probe: pipe_read failed: {:?}\n", read_res);
                return;
            }
            crate::pipe::close_read_end(pipe_id);
            crate::pipe::close_write_end(pipe_id);
            pr_info!("features_boot_probe: pipe ring buffer validated\n");
        }

        // 3. Test COW frame reference counting
        if let Ok(frame) = paging::alloc_physical_page() {
            if paging::frame_ref_count(frame) != 1 {
                pr_info!("features_boot_probe: initial frame refcount != 1\n");
                let _ = paging::free_physical_page(frame);
                return;
            }
            paging::frame_ref_inc(frame);
            if paging::frame_ref_count(frame) != 2 {
                pr_info!("features_boot_probe: frame_ref_inc failed\n");
                let _ = paging::free_physical_page(frame);
                return;
            }
            let new_c = paging::frame_ref_dec(frame);
            if new_c != 1 || paging::frame_ref_count(frame) != 1 {
                pr_info!("features_boot_probe: frame_ref_dec failed\n");
                let _ = paging::free_physical_page(frame);
                return;
            }
            let _ = paging::free_physical_page(frame);
            pr_info!("features_boot_probe: COW frame reference counting validated\n");
        }

        // 4. Test /proc virtual filesystem resolution & reading
        let root = fs::ROOT_NODE;
        if !root.is_null() {
            if let Ok(uptime_node) = fs::resolve_path("/proc/uptime", root) {
                let mut ubuf = [0u8; 32];
                if let Ok(bytes) = (*uptime_node).read(&mut ubuf, 0) {
                    if bytes > 0 {
                        pr_info!(
                            "features_boot_probe: /proc dynamic read validated ({} bytes)\n",
                            bytes
                        );
                    }
                }
            }
        }
    }
}

crate::late_initcall!(kernel_features_boot_probe);

#[unsafe(no_mangle)]
pub(crate) unsafe fn process_socket() {
    let mut fd: u32;
    core::arch::asm!(
        "int 0x80",
        in("eax") 97,
        in("ebx") 1, // AF_UNIX
        in("ecx") 1, // SOCK_STREAM
        in("edx") 0, // protocol
        lateout("eax") fd,
        options(nostack, nomem)
    );

    let out_msg: [u8; 5] = [b'I', b'P', b'C', b'!', b'\n'];
    core::arch::asm!(
        "int 0x80",
        in("eax") 4,
        in("ebx") fd,
        in("ecx") out_msg.as_ptr(),
        in("edx") out_msg.len(),
        options(nostack),
    );

    let mut in_buffer: [u8; 8] = [0; 8];
    let mut read_size: u32;
    core::arch::asm!(
        "int 0x80",
        in("eax") 3,
        in("ebx") fd,
        in("ecx") in_buffer.as_mut_ptr(),
        in("edx") in_buffer.len(),
        lateout("eax") read_size,
        options(nostack),
    );

    core::arch::asm!(
        "int 0x80",
        in("eax") 4,
        in("ebx") 1,
        in("ecx") in_buffer.as_ptr(),
        in("edx") read_size,
        options(nostack),
    );

    core::arch::asm!("int 0x80", in("eax") 1, in("ebx") 0, options(noreturn));
}

/// Current flow:
///   1. Parent process creates a socket and forks a child process.
///   2. Child process opens the file "/test.txt" and reads 32 bytes from it,
///      then sends the data to the parent process through the socket.
///   3. Parent process waits for the child process to exit,
///      then reads the data from the socket and prints it to stdout.
///
/// Or in pseudo-code:
/// ```
///  sock_fd = socket(AF_UNIX, SOCK_STREAM, 0)
///  pid = fork()
///  if pid == 0 {
///      ext2_fd = open("/test.txt", O_RDONLY)
///      read_size = read(ext2_fd, ext2_fd_read_buf, 32)
///      write(sock_fd, ext2_fd_read_buf, ext2_fd_read_size)
///      exit(0)
///  } else if pid > 0 {
///      write(1, "Waiting\n", 8)
///      wait(NULL)
///      write(1, "Waked\n", 6)
///      read_size = read(sock_fd, parent_buf, 32)
///      write(1, parent_buf, read_size)
///      exit(0)
///  } else {
///      exit(1)
///  }
/// ```
///
#[unsafe(no_mangle)]
pub(crate) unsafe fn most_syscalls_we_have_probably() {
    let sock_fd: u32;
    let pid: u32;
    let slp_msg = [b'W', b'a', b'i', b't', b'i', b'n', b'g', b'\n'];
    let wake_msg = [b'W', b'a', b'k', b'e', b'd', b'\n'];
    let read_size: u32;
    let mut parent_buf = [0u8; 32];

    // sock_fd = socket(AF_UNIX, SOCK_STREAM, 0)
    core::arch::asm!(
        "int 0x80",
        in("eax") 97,
        in("ebx") 1, // AF_UNIX
        in("ecx") 1, // SOCK_STREAM
        in("edx") 0, // protocol
        lateout("eax") sock_fd,
        options(nostack, nomem)
    );
    // pid = fork()
    core::arch::asm!(
        "int 0x80",
        in("eax") 2,
        lateout("eax") pid,
        options(nostack)
    );
    if pid == 0 {
        let ext2_fd: u32;
        let file_name: [u8; 10] = [b'/', b't', b'e', b's', b't', b'.', b't', b'x', b't', 0];
        core::arch::asm!(
            "int 0x80",
            in("eax") 5,
            in("ebx") file_name.as_ptr(),
            in("ecx") 0,
            in("edx") 0,
            lateout("eax") ext2_fd,
            options(nostack, nomem)
        );
        let mut ext2_fd_read_buf: [u8; 32] = [0; 32];
        let mut ext2_fd_read_size: u32;
        core::arch::asm!(
            "int 0x80",
            in("eax") 3,
            in("ebx") ext2_fd,
            in("ecx") ext2_fd_read_buf.as_mut_ptr(),
            in("edx") ext2_fd_read_buf.len(),
            lateout("eax") ext2_fd_read_size,
            options(nostack)
        );
        core::arch::asm!(
            "int 0x80",
            in("eax") 4,
            in("ebx") sock_fd,
            in("ecx") ext2_fd_read_buf.as_ptr(),
            in("edx") ext2_fd_read_size,
            options(nostack)
        );
        core::arch::asm!("int 0x80", in("eax") 1, in("ebx") 0, options(noreturn));
    } else if pid > 0 {
        core::arch::asm!(
            "int 0x80",
            in("eax") 4,
            in("ebx") 1,
            in("ecx") slp_msg.as_ptr(),
            in("edx") slp_msg.len(),
            options(nostack)
        );
        core::arch::asm!("int 0x80", in("eax") 7, options(nostack));
        core::arch::asm!(
            "int 0x80",
            in("eax") 4,
            in("ebx") 1,
            in("ecx") wake_msg.as_ptr(),
            in("edx") wake_msg.len(),
            options(nostack)
        );
        core::arch::asm!(
            "int 0x80",
            in("eax") 3,
            in("ebx") sock_fd,
            in("ecx") parent_buf.as_mut_ptr(),
            in("edx") parent_buf.len(),
            lateout("eax") read_size,
            options(nostack)
        );
        core::arch::asm!(
            "int 0x80",
            in("eax") 4,
            in("ebx") 1,
            in("ecx") parent_buf.as_ptr(),
            in("edx") read_size,
            options(nostack),
        );
        core::arch::asm!("int 0x80", in("eax") 1, in("ebx") 0, options(noreturn));
    } else {
        core::arch::asm!("int 0x80", in("eax") 1, in("ebx") 1, options(noreturn));
    }
}
