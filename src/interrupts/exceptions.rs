use crate::interrupts::idt::register_interrupt_handler;
use crate::startup_config::logging::DEFAULT_LOG_SCREEN;
use crate::vga::text_mod;
use crate::{pr_emerg, pr_err, pr_warn, x86};

const EXCEPTION_NAMES: [&str; 32] = [
    "Divide Error",
    "Debug",
    "NMI",
    "Breakpoint",
    "Overflow",
    "BOUND Range Exceeded",
    "Invalid Opcode",
    "Device Not Available",
    "Double Fault",
    "Coprocessor Segment Overrun",
    "Invalid TSS",
    "Segment Not Present",
    "Stack-Segment Fault",
    "General Protection Fault",
    "Page Fault",
    "Reserved",
    "x87 Floating-Point",
    "Alignment Check",
    "Machine Check",
    "SIMD Floating-Point",
    "Virtualization",
    "Control Protection",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Hypervisor Injection",
    "VMM Communication",
    "Security",
    "Reserved",
];

unsafe extern "C" {
    fn isr_exception_0();
    fn isr_exception_1();
    fn isr_exception_2();
    fn isr_exception_3();
    fn isr_exception_4();
    fn isr_exception_5();
    fn isr_exception_6();
    fn isr_exception_7();
    fn isr_exception_8();
    fn isr_exception_9();
    fn isr_exception_10();
    fn isr_exception_11();
    fn isr_exception_12();
    fn isr_exception_13();
    fn isr_exception_14();
    fn isr_exception_15();
    fn isr_exception_16();
    fn isr_exception_17();
    fn isr_exception_18();
    fn isr_exception_19();
    fn isr_exception_20();
    fn isr_exception_21();
    fn isr_exception_22();
    fn isr_exception_23();
    fn isr_exception_24();
    fn isr_exception_25();
    fn isr_exception_26();
    fn isr_exception_27();
    fn isr_exception_28();
    fn isr_exception_29();
    fn isr_exception_30();
    fn isr_exception_31();
}

#[inline(always)]
const fn is_non_fatal_exception(vector: usize) -> bool {
    matches!(vector, 1 | 3)
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Registers {
    pub edi: u32,
    pub esi: u32,
    pub ebp: u32,
    pub esp: u32,
    pub ebx: u32,
    pub edx: u32,
    pub ecx: u32,
    pub eax: u32,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn exception_common_handler(vector: u32, regs: *const Registers) {
    let idx = vector as usize;
    let name = EXCEPTION_NAMES
        .get(idx)
        .copied()
        .unwrap_or("Unknown Exception");

    if idx == 14 {
        let fault_addr = x86::read_cr2();
        pr_emerg!("EXCEPTION #{}: {} (cr2={:#x})\n", idx, name, fault_addr);
    } else if is_non_fatal_exception(idx) {
        pr_warn!("EXCEPTION #{}: {} (continuing)\n", idx, name);
        return;
    } else {
        pr_err!("EXCEPTION #{}: {}\n", idx, name);
    }

    pr_emerg!("fatal CPU exception, halting kernel\n");
    x86::disable_interrupts();
    text_mod::out::switch_screen(DEFAULT_LOG_SCREEN);

    if !regs.is_null() {
        let r = &*regs;
        pr_emerg!(
            r#"Registers:
            EAX: {:#010x} EBX: {:#010x} ECX: {:#010x} EDX: {:#010x}
            ESI: {:#010x} EDI: {:#010x} EBP: {:#010x} ESP: {:#010x}
        "#,
            r.eax,
            r.ebx,
            r.ecx,
            r.edx,
            r.esi,
            r.edi,
            r.ebp,
            r.esp
        );
    }
    crate::panic::save_stack_trace();
    crate::panic::clean_registers_and_halt();
}

pub fn init_exceptions() {
    let handlers: [unsafe extern "C" fn(); 32] = [
        isr_exception_0,
        isr_exception_1,
        isr_exception_2,
        isr_exception_3,
        isr_exception_4,
        isr_exception_5,
        isr_exception_6,
        isr_exception_7,
        isr_exception_8,
        isr_exception_9,
        isr_exception_10,
        isr_exception_11,
        isr_exception_12,
        isr_exception_13,
        isr_exception_14,
        isr_exception_15,
        isr_exception_16,
        isr_exception_17,
        isr_exception_18,
        isr_exception_19,
        isr_exception_20,
        isr_exception_21,
        isr_exception_22,
        isr_exception_23,
        isr_exception_24,
        isr_exception_25,
        isr_exception_26,
        isr_exception_27,
        isr_exception_28,
        isr_exception_29,
        isr_exception_30,
        isr_exception_31,
    ];

    for (vector, handler) in handlers.iter().enumerate() {
        register_interrupt_handler(vector as u8, *handler);
    }
}
