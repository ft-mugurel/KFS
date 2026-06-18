use crate::interrupts::exceptions::Registers;

unsafe extern "C" {
    fn isr_syscall();
}

const SYSCALL_VECTOR: u8 = 0x80;

const MAX_SYSCALL_NUMBER: usize = 256;

type SyscallHandler = fn(*mut Registers);

static mut SYSCALL_ENTRIES: [Option<SyscallHandler>; MAX_SYSCALL_NUMBER] = {
    let mut table = [None; MAX_SYSCALL_NUMBER];
    table[4] = Some(syscall_write as SyscallHandler);

    table
};

fn syscall_write(regs: *mut Registers) {
    let fd = unsafe { (*regs).ebx };
    let buf_ptr = unsafe { (*regs).ecx as *const u8 };
    let count = unsafe { (*regs).edx };
    crate::pr_debug!(
        "Syscall: write(fd={}, buf_ptr={:?}, count={})\n",
        fd,
        buf_ptr,
        count
    );
    if fd == 1 {
        let slice = unsafe { core::slice::from_raw_parts(buf_ptr, count as usize) };
        let s = core::str::from_utf8(slice).unwrap_or("<invalid utf8>");
        crate::pr_info!("Syscall write output: {}\n", s);
        unsafe { (*regs).eax = count };
    } else {
        crate::pr_warn!("Syscall write: We don't have fd's yet: {}\n", fd);
        unsafe { (*regs).eax = !0u32 }; // somewhat -1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_dispatcher(regs: *mut Registers) {

    let syscall_no = unsafe { (*regs).eax } as usize;
    if syscall_no < MAX_SYSCALL_NUMBER {
        // 2. O(1) lookup: execute the handler if it exists
        if let Some(handler) = SYSCALL_ENTRIES[syscall_no] {
            handler(regs);
            return;
        }
    }

    crate::pr_warn!("Unknown or unimplemented syscall number: {}\n", syscall_no);
    unsafe { (*regs).eax = (!0u32) - 38 + 1 };
}

pub fn init_syscalls() {
    crate::interrupts::idt::register_user_interrupt_handler(SYSCALL_VECTOR, isr_syscall);
}
