use core::arch::asm;

/// Write 8 bits to port
///
/// # Safety
/// Needs IO privileges.
#[inline(always)]
pub fn outb(port: u16, val: u8) {
    unsafe {
        asm!("outb %al, %dx", in("al") val, in("dx") port, options(att_syntax));
    }
}

/// Read 8 bits from port
///
/// # Safety
/// Needs IO privileges.
#[inline(always)]
pub fn inb(port: u16) -> u8 {
    let ret: u8;
    unsafe {
        asm!("inb %dx, %al", in("dx") port, out("al") ret, options(att_syntax));
    }
    ret
}

/// Write 16 bits to port
///
/// # Safety
/// Needs IO privileges.
#[inline(always)]
pub fn outw(port: u16, val: u16) {
    unsafe {
        asm!("outw %ax, %dx", in("ax") val, in("dx") port, options(att_syntax));
    }
}

/// Read 16 bits from port
///
/// # Safety
/// Needs IO privileges.
#[inline(always)]
#[allow(dead_code)]
pub fn inw(port: u16) -> u16 {
    let ret: u16;
    unsafe {
        asm!("inw %dx, %ax", in("dx") port, out("ax") ret, options(att_syntax));
    }
    ret
}

/// Write 32 bits to port
///
/// # Safety
/// Needs IO privileges.
#[inline(always)]
#[allow(dead_code)]
pub fn outl(port: u16, val: u32) {
    unsafe {
        asm!("outl %eax, %dx", in("eax") val, in("dx") port, options(att_syntax));
    }
}

/// Read 32 bits from port
///
/// # Safety
/// Needs IO privileges.
#[inline(always)]
#[allow(dead_code)]
pub fn inl(port: u16) -> u32 {
    let ret: u32;
    unsafe {
        asm!("inl %dx, %eax", out("eax") ret, in("dx") port, options(att_syntax));
    }
    ret
}

#[inline(always)]
pub(crate) fn disable_interrupts() {
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
    }
}

pub(crate) fn enable_interrupts() {
    unsafe {
        asm!("sti", options(nostack, preserves_flags)); // Enable interrupts
    }
}

#[inline(always)]
pub fn read_cr0() -> u32 {
    let value: u32;
    unsafe {
        asm!("mov {0:e}, cr0", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}

#[inline(always)]
pub fn write_cr0(value: u32) {
    unsafe {
        asm!("mov cr0, {0:e}", in(reg) value, options(nomem, nostack, preserves_flags));
    }
}

#[inline(always)]
pub fn read_cr3() -> u32 {
    let value: u32;
    unsafe {
        asm!("mov {0:e}, cr3", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}

#[inline(always)]
pub fn read_cr2() -> u32 {
    let value: u32;
    unsafe {
        asm!("mov {0:e}, cr2", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}

#[inline(always)]
pub fn write_cr3(value: u32) {
    unsafe {
        asm!("mov cr3, {0:e}", in(reg) value, options(nomem, nostack, preserves_flags));
    }
}

#[inline(always)]
pub fn read_esp() -> u32 {
    let value: u32;
    unsafe {
        asm!("mov {0:e}, esp", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}

#[inline(always)]
pub fn read_ebp() -> u32 {
    let value: u32;
    unsafe {
        asm!("mov {0:e}, ebp", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}

#[inline(always)]
#[unsafe(no_mangle)]
pub fn enable_paging() {
    const CR0_PG: u32 = 1 << 31;
    let cr0 = read_cr0();
    write_cr0(cr0 | CR0_PG);
}

#[inline(always)]
pub unsafe fn invalidate_page(virt_addr: u32) {
    core::arch::asm!("invlpg [{}]", in(reg) virt_addr, options(nostack, preserves_flags));
}

#[inline(always)]
pub unsafe fn read_eflags() -> u32 {
    let eflags: u32;
    // pushfd pushes EFLAGS onto the stack, pop retrieves it
    asm!("pushfd", "pop {}", out(reg) eflags, options(nomem, preserves_flags));
    eflags
}

#[inline(always)]
pub unsafe fn restore_eflags(flags: u32) {
    // The IF (Interrupt Enable Flag) is bit 9 in the EFLAGS register
    if (flags & (1 << 9)) != 0 {
        asm!("sti", options(nomem, nostack));
    }
}

#[inline(always)]
pub fn hlt() {
    unsafe {
        asm!("hlt", options(nomem, nostack));
    }
}

#[inline(always)]
pub unsafe fn clean_registers_and_halt() -> ! {
    asm!(
        "xor eax, eax",
        "xor ebx, ebx",
        "xor ecx, ecx",
        "xor edx, edx",
        "xor esi, esi",
        "xor edi, edi",
        "xor ebp, ebp",
        "cli",
        "2:",
        "hlt",
        "jmp 2b",
        options(noreturn, nostack)
    );
}

#[inline(always)]
pub fn io_wait() {
    outb(0x80, 0);
}

/// Only to be used in early boot code, before the APIC is initialized.
/// This is a busy wait that takes approximately 1 microsecond.
pub fn busy_wait_us(microseconds: u32) {
    for _ in 0..microseconds {
        io_wait();
    }
}

#[inline(always)]
pub fn rdmsr(msr: u32) -> u64 {
    let (low, high): (u32, u32);
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags)
        );
    }
    ((high as u64) << 32) | (low as u64)
}

#[inline(always)]
pub fn wrmsr(msr: u32, value: u64) {
    let low = value as u32;
    let high = (value >> 32) as u32;
    unsafe {
        asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") low,
            in("edx") high,
            options(nomem, nostack, preserves_flags)
        );
    }
}
