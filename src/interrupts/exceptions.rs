use crate::interrupts::register_interrupt_handler;
use crate::paging;
use crate::panic;
use crate::sched::{self, ProcessState};
use crate::startup_config::logging::DEFAULT_LOG_SCREEN;
use crate::vga::text_mod;
use crate::x86;
use crate::{pr_emerg, pr_err, pr_info, pr_warn};

const EXCEPTION_NAMES: [&str; 32] = [
    /* 0x00 */ "Divide Error",
    /* 0x01 */ "Debug",
    /* 0x02 */ "NMI",
    /* 0x03 */ "Breakpoint",
    /* 0x04 */ "Overflow",
    /* 0x05 */ "BOUND Range Exceeded",
    /* 0x06 */ "Invalid Opcode",
    /* 0x07 */ "Device Not Available",
    /* 0x08 */ "Double Fault",
    /* 0x09 */ "Coprocessor Segment Overrun",
    /* 0x0A */ "Invalid TSS",
    /* 0x0B */ "Segment Not Present",
    /* 0x0C */ "Stack-Segment Fault",
    /* 0x0D */ "General Protection Fault",
    /* 0x0E */ "Page Fault",
    /* 0x0F */ "Reserved",
    /* 0x10 */ "x87 Floating-Point",
    /* 0x11 */ "Alignment Check",
    /* 0x12 */ "Machine Check",
    /* 0x13 */ "SIMD Floating-Point",
    /* 0x14 */ "Virtualization",
    /* 0x15 */ "Control Protection",
    /* 0x16 */ "Reserved",
    /* 0x17 */ "Reserved",
    /* 0x18 */ "Reserved",
    /* 0x19 */ "Reserved",
    /* 0x1A */ "Reserved",
    /* 0x1B */ "Reserved",
    /* 0x1C */ "Hypervisor Injection",
    /* 0x1D */ "VMM Communication",
    /* 0x1E */ "Security",
    /* 0x1F */ "Reserved",
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
    fn restore_context_and_iret(new_esp: u32) -> !;
}

#[inline(always)]
const fn is_non_fatal_exception(vector: usize) -> bool {
    matches!(vector, 1 | 3)
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ExceptionStackFrame {
    pub edi: u32,
    pub esi: u32,
    pub ebp: u32,
    pub esp: u32,
    pub ebx: u32,
    pub edx: u32,
    pub ecx: u32,
    pub eax: u32,

    pub gs: u32,
    pub fs: u32,
    pub es: u32,
    pub ds: u32,

    pub error_code: u32,
    pub eip: u32,
    pub cs: u32,
    pub eflags: u32,
    pub user_esp: u32,
    pub user_ss: u32,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn exception_common_handler(vector: u32, regs: *const ExceptionStackFrame) {
    let idx = vector as usize;
    let name = EXCEPTION_NAMES
        .get(idx)
        .copied()
        .unwrap_or("Unknown Exception");
    let frame = &*regs;
    let fault_cr2 = if vector == 14 { x86::read_cr2() } else { 0 };
    pr_emerg!(
        "[EXC] Exception #{} ({}) at EIP={:#x}, CS={:#x}, err={:#x}, cr2={:#x}, esp={:#x}\n",
        vector,
        name,
        frame.eip,
        frame.cs,
        frame.error_code,
        fault_cr2,
        frame.esp
    );
    let task_opt = sched::current().as_mut();

    if (frame.cs & 0x03) == 3 && task_opt.is_none() {
        pr_emerg!(
            "User-mode exception {} (vector {}) occurred, but no current task found. Halting.\n",
            name,
            idx
        );
    } else if (frame.cs & 0x03) == 3 {
        let current_pid = sched::current_pid();
        let idx_usize = vector as usize;

        let task = task_opt.unwrap();

        if idx_usize == 14 {
            let fault_addr = x86::read_cr2();
            let mem = &task.memory;

            let mut is_valid = false;
            let mut flags = paging::PAGE_PRESENT | paging::PAGE_USER;

            if fault_addr >= mem.code_base && fault_addr < mem.code_base + mem.code_size {
                is_valid = true;
                flags |= paging::PAGE_WRITABLE;
            } else if fault_addr >= mem.data_base && fault_addr < mem.data_base + mem.data_size {
                is_valid = true;
                flags |= paging::PAGE_WRITABLE;
            } else if fault_addr >= mem.bss_base && fault_addr < mem.bss_base + mem.bss_size {
                is_valid = true;
                flags |= paging::PAGE_WRITABLE;
            } else if fault_addr >= mem.stack_limit && fault_addr <= mem.stack_base {
                is_valid = true;
                flags |= paging::PAGE_WRITABLE;
            } else if fault_addr >= mem.heap_base && fault_addr < mem.heap_brk {
                is_valid = true;
                flags |= paging::PAGE_WRITABLE;
            } else {
                for vma in &mem.vmas {
                    if vma.used && fault_addr >= vma.base && fault_addr < vma.base + vma.size {
                        is_valid = true;
                        flags |= paging::PAGE_WRITABLE;
                        break;
                    }
                }
            }

            if is_valid {
                let aligned_vaddr = fault_addr & !0xFFF;
                match paging::map_zero_page(aligned_vaddr, flags) {
                    Ok(()) => {
                        pr_info!(
                            "Demand paging: Mapped page for PID {} at {:#x}\n",
                            current_pid,
                            aligned_vaddr
                        );
                        core::ptr::write_bytes(aligned_vaddr as *mut u8, 0, 4096);
                        return;
                    }
                    Err(e) => {
                        pr_err!("OOM: Cannot demand page PID {}: {:?}\n", current_pid, e);
                    }
                }
            } else {
                pr_err!("Segmentation Fault at {:#x}\n", fault_addr);
            }
        }

        // If it was not a handled page fault, terminate the process
        let sig_num = match idx_usize {
            0 => 8,        // Divide by Zero -> SIGFPE
            6 => 4,        // Invalid Opcode -> SIGILL
            13 | 14 => 11, // GPF or Unhandled Page Fault -> SIGSEGV
            _ => 9,        // Unknown fatal fault -> SIGKILL
        };

        pr_info!(
            "PID {} killed by hardware exception {} (Signal {})\n",
            current_pid,
            idx_usize,
            sig_num
        );

        task.state = ProcessState::Zombie;

        x86::write_cr3(paging::bootstrap_directory_phys_addr());
        let next_esp = sched::schedule(0);
        restore_context_and_iret(next_esp);
    }

    if idx == 14 || idx == 13 {
        let present = (frame.error_code & 0b001) != 0;
        let write = (frame.error_code & 0b010) != 0;
        let user = (frame.error_code & 0b100) != 0;
        let fault_addr = x86::read_cr2();
        pr_emerg!("EXCEPTION #{}: {} (cr2={:#x})\n", idx, name, fault_addr);
        pr_emerg!(
            "Page Fault Details: present={}, write={}, user={}\n",
            present,
            write,
            user
        );
    } else if is_non_fatal_exception(idx) {
        pr_warn!("EXCEPTION #{}: {} (continuing)\n", idx, name);
        return;
    } else {
        pr_err!("EXCEPTION #{}: {}\n", idx, name);
    }

    pr_emerg!("fatal CPU exception, halting kernel\n");
    x86::disable_interrupts();
    text_mod::switch_screen(DEFAULT_LOG_SCREEN);

    pr_emerg!(
        "Registers:\n\
        EAX: {:#010x} EBX: {:#010x} ECX: {:#010x} EDX: {:#010x}\n\
        ESI: {:#010x} EDI: {:#010x} EBP: {:#010x} ESP: {:#010x}\n\
        EIP: {:#010x} CS:  {:#010x} EFLAGS: {:#010x}\n",
        frame.eax,
        frame.ebx,
        frame.ecx,
        frame.edx,
        frame.esi,
        frame.edi,
        frame.ebp,
        frame.esp,
        frame.eip,
        frame.cs,
        frame.eflags
    );

    panic::save_stack_trace();
    panic::clean_registers_and_halt();
}

#[unsafe(link_section = ".init.text")]
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
