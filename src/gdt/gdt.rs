use core::arch::asm;
use core::mem::size_of;

use super::{KERNEL_CODE_SEL, KERNEL_DATA_SEL, TSS_SEL};

#[repr(C, packed)]
#[derive(Copy, Clone)]
struct GdtEntry {
    pub limit0: u16,
    pub base0: u16,
    pub base1_flags: u16,
    pub limit1_flags_base2: u16,
}

#[repr(C, packed)]
struct GdtPointer {
    pub limit: u16,
    pub base: u32,
}

#[repr(C, packed)]
pub struct TaskStateSegment {
    pub prev_tss: u32,
    pub esp0: u32, // The kernel stack pointer
    pub ss0: u32,  // The kernel stack segment
    pub esp1: u32,
    pub ss1: u32,
    pub esp2: u32,
    pub ss2: u32,
    pub cr3: u32,
    pub eip: u32,
    pub eflags: u32,
    pub eax: u32,
    pub ecx: u32,
    pub edx: u32,
    pub ebx: u32,
    pub esp: u32,
    pub ebp: u32,
    pub esi: u32,
    pub edi: u32,
    pub es: u32,
    pub cs: u32,
    pub ss: u32,
    pub ds: u32,
    pub fs: u32,
    pub gs: u32,
    pub ldt: u32,
    pub trap: u16,
    pub iomap_base: u16,
}

impl TaskStateSegment {
    pub const fn new() -> Self {
        Self {
            prev_tss: 0,
            esp0: 0,
            ss0: 0,
            esp1: 0,
            ss1: 0,
            esp2: 0,
            ss2: 0,
            cr3: 0,
            eip: 0,
            eflags: 0,
            eax: 0,
            ecx: 0,
            edx: 0,
            ebx: 0,
            esp: 0,
            ebp: 0,
            esi: 0,
            edi: 0,
            es: 0,
            cs: 0,
            ss: 0,
            ds: 0,
            fs: 0,
            gs: 0,
            ldt: 0,
            trap: 0,
            iomap_base: size_of::<TaskStateSegment>() as u16,
        }
    }
}

const GDT_ENTRIES_COUNT: usize = 8;
const GDT_LIMIT_BYTES: u32 = 0xfffff; // 4GiB
const GDT_LIMIT: u32 = (GDT_LIMIT_BYTES >> 12) - 1;

// Mirror Linux arch/x86/include/asm/desc_defs.h flags.
const _DESC_ACCESSED: u16 = 0x0001;
const _DESC_DATA_WRITABLE: u16 = 0x0002;
const _DESC_CODE_READABLE: u16 = 0x0002;
const _DESC_DATA_EXPAND_DOWN: u16 = 0x0004;
const _DESC_CODE_EXECUTABLE: u16 = 0x0008;
const _DESC_TSS_AVAIL: u16 = 0x0009;
const _DESC_S: u16 = 0x0010;
const _DESC_PRESENT: u16 = 0x0080;
const _DESC_DPL3: u16 = 3 << 5;
const _DESC_DB: u16 = 0x4000;
const _DESC_GRANULARITY_4K: u16 = 0x8000;

const DESC_DATA32: u16 = _DESC_S
    | _DESC_PRESENT
    | _DESC_ACCESSED
    | _DESC_DATA_WRITABLE
    | _DESC_GRANULARITY_4K
    | _DESC_DB;
const DESC_CODE32: u16 = _DESC_S
    | _DESC_PRESENT
    | _DESC_ACCESSED
    | _DESC_CODE_READABLE
    | _DESC_CODE_EXECUTABLE
    | _DESC_GRANULARITY_4K
    | _DESC_DB;
const DESC_STACK32: u16 = DESC_DATA32 | _DESC_DATA_EXPAND_DOWN;
const DESC_USER_DATA32: u16 = DESC_DATA32 | _DESC_DPL3;
const DESC_USER_CODE32: u16 = DESC_CODE32 | _DESC_DPL3;
const DESC_USER_STACK32: u16 = DESC_STACK32 | _DESC_DPL3;
const DESC_TSS32: u16 = _DESC_PRESENT | _DESC_TSS_AVAIL;

const fn make_entry(flags: u16, base: u32, limit: u32) -> GdtEntry {
    // Equivalent to Linux's GDT_ENTRY_INIT(flags, base, limit).
    GdtEntry {
        limit0: ((limit >> 0) & 0xFFFF) as u16,
        base0: ((base >> 0) & 0xFFFF) as u16,
        base1_flags: (((base >> 16) & 0x00FF) as u16) | ((flags & 0x00FF) << 8),
        limit1_flags_base2: (((limit >> 16) & 0x000F) as u16)
            | ((flags >> 8) & 0x00F0)
            | ((((base >> 24) & 0x00FF) as u16) << 8),
    }
}

#[unsafe(link_section = ".gdt")]
#[used]
static mut GDT: [GdtEntry; GDT_ENTRIES_COUNT] = [
    GdtEntry {
        limit0: 0,
        base0: 0,
        base1_flags: 0,
        limit1_flags_base2: 0,
    }, // Null segment
    make_entry(DESC_CODE32, 0, GDT_LIMIT),       // Kernel code
    make_entry(DESC_DATA32, 0, GDT_LIMIT),       // Kernel data
    make_entry(DESC_STACK32, 0, GDT_LIMIT),      // Kernel stack (expand-down data)
    make_entry(DESC_USER_CODE32, 0, GDT_LIMIT),  // User code
    make_entry(DESC_USER_DATA32, 0, GDT_LIMIT),  // User data
    make_entry(DESC_USER_STACK32, 0, GDT_LIMIT), // User stack (expand-down data)
    make_entry(DESC_TSS32, 0, 0),                // TSS
];

pub(crate) static mut TSS: TaskStateSegment = TaskStateSegment::new();

pub fn load_gdt() {
    unsafe {
        let tss_base = &raw const TSS as u32;
        let tss_limit = (size_of::<TaskStateSegment>() - 1) as u32;
        GDT[7] = make_entry(DESC_TSS32, tss_base, tss_limit);

        TSS.ss0 = KERNEL_DATA_SEL as u32; // The CPU will switch to this segment on an interrupt
        TSS.iomap_base = size_of::<TaskStateSegment>() as u16; // Prevent ring 3 from using `in/out` instructions directly

        let gdt_ptr = GdtPointer {
            limit: (size_of::<[GdtEntry; GDT_ENTRIES_COUNT]>() - 1) as u16,
            base: &raw const GDT as u32,
        };

        asm!(
            "lgdt [{}]",
            in(reg) &gdt_ptr,
            options(nostack, preserves_flags)
        );

        asm!(
            "mov ds, {kd:x}",
            "mov es, {kd:x}",
            "mov fs, {kd:x}",
            "mov gs, {kd:x}",
            "mov ss, {kd:x}",

            "push {kc}",
            "lea eax, [2f]",
            "push eax",
            "retf",
            "2:",
            kd = in(reg) KERNEL_DATA_SEL,
            kc = const KERNEL_CODE_SEL,
            out("eax") _,
        );

        asm!(
            "ltr ax",
            in("ax") TSS_SEL,
            options(nostack, preserves_flags)
        );
    }
}

pub fn set_kernel_stack(stack_top: u32) {
    unsafe {
        TSS.esp0 = stack_top;
    }
}
