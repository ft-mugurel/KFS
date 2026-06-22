use crate::sched::task::ContextFrame;

use super::close::syscall_close;
use super::exit::{syscall_exit, syscall_wait};
use super::fork::syscall_fork;
use super::mem::{syscall_mmap, syscall_munmap, syscall_sbrk};
use super::read_write::{syscall_read, syscall_write};
use super::signal::{syscall_kill, syscall_signal};
use super::socket::syscall_socket;
use super::sys::syscall_getuid;
use super::time::syscall_nanosleep;

unsafe extern "C" {
    fn isr_syscall();
}

const SYSCALL_VECTOR: u8 = 0x80;
const MAX_SYSCALL_NUMBER: usize = 256;

type SyscallHandler = unsafe fn(*mut ContextFrame);

static mut SYSCALL_ENTRIES: [Option<SyscallHandler>; MAX_SYSCALL_NUMBER] = {
    let mut table = [None; MAX_SYSCALL_NUMBER];
    table[1] = Some(syscall_exit as SyscallHandler);
    table[2] = Some(syscall_fork as SyscallHandler);
    table[3] = Some(syscall_read as SyscallHandler);
    table[4] = Some(syscall_write as SyscallHandler);
    table[6] = Some(syscall_close as SyscallHandler);
    table[7] = Some(syscall_wait as SyscallHandler);
    table[24] = Some(syscall_getuid as SyscallHandler);
    table[37] = Some(syscall_kill as SyscallHandler);
    table[45] = Some(syscall_sbrk as SyscallHandler);
    table[48] = Some(syscall_signal as SyscallHandler);
    table[90] = Some(syscall_mmap as SyscallHandler);
    table[91] = Some(syscall_munmap as SyscallHandler);
    table[97] = Some(syscall_socket as SyscallHandler);
    table[162] = Some(syscall_nanosleep as SyscallHandler);

    table
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_dispatcher(regs: *mut ContextFrame) {
    let syscall_no = unsafe { (*regs).eax } as usize;
    if syscall_no < MAX_SYSCALL_NUMBER {
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
