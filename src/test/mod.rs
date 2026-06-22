#[unsafe(no_mangle)]
pub(crate) unsafe fn process_sleep() {
    loop {
        let msg: [u8; 9] = [b'A', b'l', b'i', b'v', b'e', b'.', b'.', b'.', b'\n'];
        core::arch::asm!(
            "int 0x80",
            in("eax") 4,
            in("ebx") 1,
            in("ecx") msg.as_ptr(),
            in("edx") msg.len(),
            options(nostack),
        );
        core::arch::asm!(
            "int 0x80",
            in("eax") 162,
            in("ebx") 2000,
            options(nostack, nomem),
        );
    }
}

#[unsafe(no_mangle)]
pub(crate) unsafe fn process_socket() {
    let mut fd: u32;
    core::arch::asm!(
        "int 0x80",
        in("eax") 97,
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

#[unsafe(no_mangle)]
pub(crate) unsafe fn process_fork() {
    let fd: u32;
    let pid: u32;
    core::arch::asm!(
        "int 0x80",
        in("eax") 97,
        lateout("eax") fd,
        options(nostack, nomem)
    );
    core::arch::asm!(
        "int 0x80",
        in("eax") 2,
        lateout("eax") pid,
        options(nostack)
    );

    if pid == 0 {
        let child_msg = [b'C', b'h', b'i', b'l', b'd', b'\n', 0];
        core::arch::asm!(
            "int 0x80",
            in("eax") 4,
            in("ebx") fd,
            in("ecx") child_msg.as_ptr(),
            in("edx") child_msg.len(),
            options(nostack)
        );
    } else if pid > 0 {
        let slp_msg = [b'S', b'l', b'e', b'e', b'p', b'i', b'n', b'g', b'\n'];
        core::arch::asm!(
            "int 0x80",
            in("eax") 4,
            in("ebx") 1,
            in("ecx") slp_msg.as_ptr(),
            in("edx") slp_msg.len(),
            options(nostack)
        );
        core::arch::asm!(
            "int 0x80",
            in("eax") 162,
            in("ebx") 1000,
            options(nostack, nomem),
        );
        let wake_msg = [b'W', b'a', b'k', b'e', b'd', b'\n'];
        core::arch::asm!(
            "int 0x80",
            in("eax") 4,
            in("ebx") 1,
            in("ecx") wake_msg.as_ptr(),
            in("edx") wake_msg.len(),
            options(nostack)
        );
        let read_size: u32;
        let parent_buf = [0; 7];
        core::arch::asm!(
            "int 0x80",
            in("eax") 3,
            in("ebx") fd,
            in("ecx") parent_buf.as_ptr(),
            in("edx") parent_buf.len(),
            lateout("eax") read_size,
            options(nostack)
        );
        core::arch::asm!("int 0x80", in("eax") 7, options(nostack));

        core::arch::asm!(
            "int 0x80",
            in("eax") 4,
            in("ebx") 1,
            in("ecx") parent_buf.as_ptr(),
            in("edx") read_size,
            options(nostack),
        );
    } else {
        crate::pr_err!("Fork failed with error code: {}\n", pid);
        core::arch::asm!("int 0x80", in("eax") 1, in("ebx") 1, options(noreturn));
    }
}
