use crate::{
    dump::lookup,
    error::KernelError,
    interrupts::register_user_interrupt_handler,
    pr_debug, pr_warn,
    sched::{self, ContextFrame},
};

use super::{
    chdir::{syscall_chdir, syscall_getcwd},
    debug::syscall_debug,
    exit::{syscall_exit, syscall_wait},
    fork::syscall_fork,
    fs::{syscall_getdents, syscall_mknod, syscall_mount, syscall_umount, syscall_unlink},
    info::{syscall_getpid, syscall_getppid},
    mem::{syscall_mmap, syscall_munmap, syscall_sbrk},
    open::{syscall_close, syscall_dup2, syscall_open},
    pipe::syscall_pipe,
    read_write::{syscall_read, syscall_write},
    signal::{syscall_kill, syscall_signal},
    socket::syscall_socket,
    sys::{syscall_getttyname, syscall_getuid, syscall_getusername, syscall_login},
    time::syscall_nanosleep,
};

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
    table[10] = Some(SyscallEntry::Normal(syscall_unlink)); // unlink(path)
    table[12] = Some(SyscallEntry::Normal(syscall_chdir)); // chdir(path)
    table[14] = Some(SyscallEntry::Normal(syscall_mknod)); // mknod(path, mode)
    table[21] = Some(SyscallEntry::Normal(syscall_mount)); // mount(source, target, fstype, flags, data)
    table[52] = Some(SyscallEntry::Normal(syscall_umount)); // umount(target)
    table[63] = Some(SyscallEntry::Normal(syscall_dup2)); // dup2(old_fd, new_fd)
    table[141] = Some(SyscallEntry::Normal(syscall_getdents)); // getdents(fd, dirp, count)
    table[183] = Some(SyscallEntry::Normal(syscall_getcwd)); // getcwd(buf, size)

    // Debugging & system control
    table[223] = Some(SyscallEntry::Normal(syscall_debug)); // debug(op, ...)

    // System info & auth
    table[20] = Some(SyscallEntry::Normal(syscall_getpid)); // getpid()
    table[64] = Some(SyscallEntry::Normal(syscall_getppid)); // getppid()
    table[199] = Some(SyscallEntry::Normal(syscall_getuid)); // getuid()
    table[212] = Some(SyscallEntry::Normal(syscall_login)); // login(user, pass)
    table[213] = Some(SyscallEntry::Normal(syscall_getusername)); // getusername(buf, size)
    table[214] = Some(SyscallEntry::Normal(syscall_getttyname)); // getttyname(fd, buf, size)

    // Signals
    table[37] = Some(SyscallEntry::Normal(syscall_kill)); // kill(pid, sig)
    table[42] = Some(SyscallEntry::Normal(syscall_pipe)); // pipe(pipefd)
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
    crate::modules::run_deferred_work();
    let task = sched::current().as_mut().unwrap();
    task.context.esp = regs as u32;

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

            let result = entry.invoke(regs);

            if (*regs).is_error() {
                pr_debug!(
                    "Syscall {} returned error: {:?}\n",
                    syscall_name,
                    (*regs).get_error().unwrap_or(KernelError::E2BIG)
                );
            } else {
                pr_debug!(
                    "Syscall {} returned value: {}\n",
                    syscall_name,
                    (*regs).get_return_value()
                );
            }
            if result != 0 {
                return result;
            }
            if task.state == sched::ProcessState::Waiting {
                return sched::schedule(regs as u32);
            }
            return 0;
        }
    }

    pr_warn!("Unknown syscall number: {}\n", syscall_no);
    (*regs).set_return_error(KernelError::ENOSYS);
    0
}

#[unsafe(link_section = ".init.text")]
pub fn init_syscalls() {
    register_user_interrupt_handler(SYSCALL_VECTOR, isr_syscall);
}

crate::core_initcall!(init_syscalls);
