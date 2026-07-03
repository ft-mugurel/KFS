use crate::dump::lookup;
use crate::error::KernelError;
use crate::interrupts::register_user_interrupt_handler;
use crate::sched::{ContextFrame, CURRENT_PID, PROCESS_TABLE};
use crate::{pr_debug, pr_warn};

use super::exit::{syscall_exit, syscall_wait};
use super::fork::syscall_fork;
use super::mem::{syscall_mmap, syscall_munmap, syscall_sbrk};
use super::open::{syscall_close, syscall_open};
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
type ReschedulingSyscallHandler = unsafe fn(*mut ContextFrame) -> u32;

enum SyscallEntry {
    Normal(SyscallHandler),
    Rescheduling(ReschedulingSyscallHandler),
}

impl SyscallEntry {
    unsafe fn invoke(&self, regs: *mut ContextFrame) -> u32 {
        match self {
            SyscallEntry::Normal(handler) => {
                handler(regs);
                0
            }
            SyscallEntry::Rescheduling(handler) => handler(regs),
        }
    }
}

static mut SYSCALL_ENTRIES: [Option<SyscallEntry>; MAX_SYSCALL_NUMBER] = {
    const NONE: Option<SyscallEntry> = None;
    let mut table: [Option<SyscallEntry>; MAX_SYSCALL_NUMBER] = [NONE; MAX_SYSCALL_NUMBER];

    // Process management
    table[2] = Some(SyscallEntry::Normal(syscall_fork)); // fork()
    table[7] = Some(SyscallEntry::Normal(syscall_wait)); // wait()

    // File I/O
    table[3] = Some(SyscallEntry::Normal(syscall_read)); // read(fd, buf, len)
    table[4] = Some(SyscallEntry::Normal(syscall_write)); // write(fd, buf, len)
    table[5] = Some(SyscallEntry::Normal(syscall_open)); // open(path, flags)
    table[6] = Some(SyscallEntry::Normal(syscall_close)); // close(fd)

    // System info
    table[24] = Some(SyscallEntry::Normal(syscall_getuid)); // getuid()

    // Signals
    table[37] = Some(SyscallEntry::Normal(syscall_kill)); // kill(pid, sig)
    table[48] = Some(SyscallEntry::Normal(syscall_signal)); // signal(sig, handler)

    // Memory
    table[45] = Some(SyscallEntry::Normal(syscall_sbrk)); // sbrk(increment)
    table[90] = Some(SyscallEntry::Normal(syscall_mmap)); // mmap(...)
    table[91] = Some(SyscallEntry::Normal(syscall_munmap)); // munmap(...)

    // IPC & Networking
    table[97] = Some(SyscallEntry::Normal(syscall_socket)); // socket(family, type, proto)

    // Timing
    table[162] = Some(SyscallEntry::Normal(syscall_nanosleep)); // nanosleep(ms)

    // Can trigger context switch by returning the next ESP

    table[1] = Some(SyscallEntry::Rescheduling(
        syscall_exit as ReschedulingSyscallHandler,
    ));

    table
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_dispatcher(regs: *mut ContextFrame) -> u32 {
    let current_pid = CURRENT_PID;

    if let Some(ref mut task) = PROCESS_TABLE[current_pid] {
        task.context.esp = regs as u32;
    }

    let syscall_no = unsafe { (*regs).eax } as usize;
    if syscall_no < MAX_SYSCALL_NUMBER {
        if let Some(entry) = &SYSCALL_ENTRIES[syscall_no] {
            let handler_ptr = match entry {
                SyscallEntry::Normal(h) => *h as *const (),
                SyscallEntry::Rescheduling(h) => *h as *const (),
            };

            let syscall_name_full_path = lookup(handler_ptr as u32).unwrap_or(("unknown", 0)).0;
            let syscall_name = syscall_name_full_path
                .split("::")
                .last()
                .unwrap_or("unknown");

            pr_debug!(
                "[PID {}] Syscall: {} from EIP: {:#x}\n",
                current_pid,
                syscall_name,
                (*regs).eip as u32
            );

            let result = entry.invoke(regs);

            if (*regs).is_error() {
                pr_debug!(
                    "[PID {}] Syscall {} returned error: {:?}\n",
                    current_pid,
                    syscall_name,
                    (*regs).get_error().unwrap_or(KernelError::EINVAL)
                );
            } else {
                pr_debug!(
                    "[PID {}] Syscall {} returned value: {}\n",
                    current_pid,
                    syscall_name,
                    (*regs).get_return_value()
                );
            }
            return result;
        }
    }

    pr_warn!(
        "[PID {}] Unknown syscall number: {}\n",
        current_pid,
        syscall_no
    );
    (*regs).set_return_error(KernelError::ENOSYS);
    0
}

pub fn init_syscalls() {
    register_user_interrupt_handler(SYSCALL_VECTOR, isr_syscall);
}
