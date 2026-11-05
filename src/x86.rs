use core::arch::asm;

// Port I/O blocks keep the default memory clobber, so the compiler cannot
// move buffer reads/writes across a device command.

/// # Safety
/// Needs IO privileges.
#[inline(always)]
pub unsafe fn outb(port: u16, val: u8) {
    unsafe {
        asm!("outb %al, %dx", in("al") val, in("dx") port, options(att_syntax, nostack, preserves_flags));
    }
}

/// # Safety
/// Needs IO privileges.
#[inline(always)]
pub unsafe fn inb(port: u16) -> u8 {
    let ret: u8;
    unsafe {
        asm!("inb %dx, %al", in("dx") port, out("al") ret, options(att_syntax, nostack, preserves_flags));
    }
    ret
}

/// # Safety
/// Needs IO privileges.
#[inline(always)]
pub unsafe fn outw(port: u16, val: u16) {
    unsafe {
        asm!("outw %ax, %dx", in("ax") val, in("dx") port, options(att_syntax, nostack, preserves_flags));
    }
}

/// # Safety
/// Needs IO privileges.
#[inline(always)]
pub unsafe fn inw(port: u16) -> u16 {
    let ret: u16;
    unsafe {
        asm!("inw %dx, %ax", in("dx") port, out("ax") ret, options(att_syntax, nostack, preserves_flags));
    }
    ret
}

/// # Safety
/// Needs IO privileges.
#[inline(always)]
pub unsafe fn outl(port: u16, val: u32) {
    unsafe {
        asm!("outl %eax, %dx", in("eax") val, in("dx") port, options(att_syntax, nostack, preserves_flags));
    }
}

/// # Safety
/// Needs IO privileges.
#[inline(always)]
pub unsafe fn inl(port: u16) -> u32 {
    let ret: u32;
    unsafe {
        asm!("inl %dx, %eax", out("eax") ret, in("dx") port, options(att_syntax, nostack, preserves_flags));
    }
    ret
}

#[inline(always)]
pub fn io_wait() {
    unsafe { outb(0x80, 0) };
}

/// Busy-waits for about `microseconds` µs (one port 0x80 write each).
/// Only accurate enough for early boot, before the APIC timer is calibrated.
pub fn busy_wait_us(microseconds: u32) {
    for _ in 0..microseconds {
        io_wait();
    }
}

// cli/sti/hlt keep the default memory clobber (no `nomem`): they act as
// compiler barriers, so memory accesses stay inside the interrupts-off
// section and are re-read after a wakeup, as Linux does with "memory".

#[inline(always)]
pub fn disable_interrupts() {
    unsafe {
        asm!("cli", options(nostack, preserves_flags));
    }
}

/// Enables maskable interrupts on this CPU.
///
/// # Safety
/// The caller must not be inside a section that relies on interrupts being
/// off, such as while holding a `Spinlock` or
/// touching per-CPU state an interrupt handler also uses
pub unsafe fn enable_interrupts() {
    unsafe {
        asm!("sti", options(nostack, preserves_flags));
    }
}

#[inline(always)]
pub fn read_eflags() -> u32 {
    let eflags: u32;
    unsafe {
        asm!("pushfd", "pop {}", out(reg) eflags, options(nomem, preserves_flags));
    }
    eflags
}

#[inline(always)]
pub unsafe fn restore_eflags(flags: u32) {
    if (flags & (1 << 9)) != 0 {
        unsafe {
            asm!("sti", options(nostack, preserves_flags));
        }
    }
}

#[inline(always)]
pub fn hlt() {
    unsafe {
        asm!("hlt", options(nostack, preserves_flags));
    }
}

#[inline(always)]
pub fn clean_registers_and_halt() -> ! {
    unsafe {
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
pub fn read_cr2() -> u32 {
    let value: u32;
    unsafe {
        asm!("mov {0:e}, cr2", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
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

// Control-register writes change what addresses mean, so they are unsafe
// and keep the memory clobber: no load or store may move across them.

/// # Safety
/// Must not clear PE or PG while the kernel depends on protected mode and
/// paging, and may only set PG when CR3 holds a page directory that maps the
/// code and stack currently in use.
#[inline(always)]
pub unsafe fn write_cr0(value: u32) {
    unsafe {
        asm!("mov cr0, {0:e}", in(reg) value, options(nostack, preserves_flags));
    }
}

/// # Safety
/// `value` must be the physical address of a valid page directory that maps
/// the code and stack currently in use and every kernel address the caller
/// touches afterwards.
#[inline(always)]
pub unsafe fn write_cr3(value: u32) {
    unsafe {
        asm!("mov cr3, {0:e}", in(reg) value, options(nostack, preserves_flags));
    }
}

#[unsafe(no_mangle)]
pub unsafe fn enable_paging() {
    const CR0_PG: u32 = 1 << 31;
    let cr0 = read_cr0();
    unsafe { write_cr0(cr0 | CR0_PG) };
}

/// Drops this CPU's cached translation for one page. At worst, it costs
/// a TLB refill. It keeps the memory clobber so it stays after the PTE write
/// it follows
#[inline(always)]
pub fn invalidate_page(virt_addr: u32) {
    unsafe {
        asm!("invlpg [{}]", in(reg) virt_addr, options(nostack, preserves_flags));
    }
}

#[inline(always)]
pub unsafe fn rdmsr(msr: u32) -> u64 {
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
pub unsafe fn wrmsr(msr: u32, value: u64) {
    let low = value as u32;
    let high = (value >> 32) as u32;
    unsafe {
        asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") low,
            in("edx") high,
            options(nostack, preserves_flags)
        );
    }
}
