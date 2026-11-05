use crate::{
    error::{KResult, KernelError},
    interrupts::register_user_interrupt_handler,
    pr_debug, pr_warn,
    sched::{self, ContextFrame},
};

use kfs_abi as abi;

use super::{
    args::SyscallArgs,
    chdir::{syscall_chdir, syscall_getcwd, syscall_readlink},
    debug::syscall_debug,
    exit::{syscall_exit, syscall_wait},
    fork::syscall_fork,
    fs::{
        syscall_getdents, syscall_lstat, syscall_mknod, syscall_mount, syscall_umount,
        syscall_unlink,
    },
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

/// For the few syscalls that need the frame itself (`fork` copies it, `exit`
/// never returns to it). Returns the ESP to resume on, or 0 to stay on this task.
type FrameSyscallHandler = unsafe fn(*mut ContextFrame) -> u32;
type ArgsSyscallHandler = fn(SyscallArgs) -> KResult<u32>;

enum SyscallHandler {
    Frame(FrameSyscallHandler),
    Args(ArgsSyscallHandler),
}

struct SyscallEntry {
    name: &'static str,
    handler: SyscallHandler,
}

impl SyscallEntry {
    const fn frame(name: &'static str, handler: FrameSyscallHandler) -> Option<Self> {
        Some(Self { name, handler: SyscallHandler::Frame(handler) })
    }

    const fn args(name: &'static str, handler: ArgsSyscallHandler) -> Option<Self> {
        Some(Self { name, handler: SyscallHandler::Args(handler) })
    }

    unsafe fn invoke(&self, regs: *mut ContextFrame) -> u32 {
        match self.handler {
            SyscallHandler::Frame(handler) => unsafe { handler(regs) },
            SyscallHandler::Args(handler) => {
                let frame = unsafe { &mut *regs };
                match handler(SyscallArgs::from_frame(frame)) {
                    Ok(value) => frame.set_return_value(value),
                    // Re-run `int 0x80` when the task is woken. eax still
                    // holds the syscall number, so it must not be written.
                    Err(KernelError::Restart) => frame.eip -= 2,
                    Err(error) => frame.set_return_error(error),
                }
                0
            }
        }
    }
}

static SYSCALL_ENTRIES: [Option<SyscallEntry>; MAX_SYSCALL_NUMBER] = {
    const NONE: Option<SyscallEntry> = None;
    let mut table: [Option<SyscallEntry>; MAX_SYSCALL_NUMBER] = [NONE; MAX_SYSCALL_NUMBER];

    // Process management
    table[abi::SYS_FORK as usize] = SyscallEntry::frame("fork", syscall_fork); // fork()
    table[abi::SYS_WAITPID as usize] = SyscallEntry::args("wait", syscall_wait); // wait()

    // File I/O
    table[abi::SYS_READ as usize] = SyscallEntry::args("read", syscall_read); // read(fd, buf, len)
    table[abi::SYS_WRITE as usize] = SyscallEntry::args("write", syscall_write); // write(fd, buf, len)
    table[abi::SYS_OPEN as usize] = SyscallEntry::args("open", syscall_open); // open(path, flags)
    table[abi::SYS_CLOSE as usize] = SyscallEntry::args("close", syscall_close); // close(fd)
    table[abi::SYS_UNLINK as usize] = SyscallEntry::args("unlink", syscall_unlink); // unlink(path)
    table[abi::SYS_CHDIR as usize] = SyscallEntry::args("chdir", syscall_chdir); // chdir(path)
    table[abi::SYS_MKNOD as usize] = SyscallEntry::args("mknod", syscall_mknod); // mknod(path, mode)
    table[abi::SYS_MOUNT as usize] = SyscallEntry::args("mount", syscall_mount); // mount(source, target, fstype, flags, data)
    table[abi::SYS_UMOUNT as usize] = SyscallEntry::args("umount", syscall_umount); // umount(target)
    table[abi::SYS_DUP2 as usize] = SyscallEntry::args("dup2", syscall_dup2); // dup2(old_fd, new_fd)
    table[abi::SYS_GETDENTS as usize] = SyscallEntry::args("getdents", syscall_getdents); // getdents(fd, dirp, count)
    table[abi::SYS_GETCWD as usize] = SyscallEntry::args("getcwd", syscall_getcwd); // getcwd(buf, size)
    table[abi::SYS_READLINK as usize] = SyscallEntry::args("readlink", syscall_readlink);
    table[abi::SYS_LSTAT as usize] = SyscallEntry::args("lstat", syscall_lstat); // lstat(path, buf)

    // Debugging & system control
    table[abi::SYS_DEBUG as usize] = SyscallEntry::args("debug", syscall_debug); // debug(op, ...)

    // System info & auth
    table[abi::SYS_GETPID as usize] = SyscallEntry::args("getpid", syscall_getpid); // getpid()
    table[abi::SYS_GETPPID as usize] = SyscallEntry::args("getppid", syscall_getppid); // getppid()
    table[abi::SYS_GETUID as usize] = SyscallEntry::args("getuid", syscall_getuid); // getuid()
    table[abi::SYS_LOGIN as usize] = SyscallEntry::args("login", syscall_login); // login(user, pass)
    table[abi::SYS_GETUSERNAME as usize] = SyscallEntry::args("getusername", syscall_getusername); // getusername(buf, size)
    table[abi::SYS_GETTTYNAME as usize] = SyscallEntry::args("getttyname", syscall_getttyname); // getttyname(fd, buf, size)

    // Signals
    table[abi::SYS_KILL as usize] = SyscallEntry::args("kill", syscall_kill); // kill(pid, sig)
    table[abi::SYS_PIPE as usize] = SyscallEntry::args("pipe", syscall_pipe); // pipe(pipefd)
    table[abi::SYS_SIGNAL as usize] = SyscallEntry::args("signal", syscall_signal); // signal(sig, handler)

    // Memory
    table[abi::SYS_SBRK as usize] = SyscallEntry::args("sbrk", syscall_sbrk); // sbrk(increment)
    table[abi::SYS_MMAP as usize] = SyscallEntry::args("mmap", syscall_mmap); // mmap(...)
    table[abi::SYS_MUNMAP as usize] = SyscallEntry::args("munmap", syscall_munmap); // munmap(...)

    // IPC & Networking
    table[abi::SYS_SOCKET as usize] = SyscallEntry::args("socket", syscall_socket); // socket(family, type, proto)

    // Timing
    table[abi::SYS_NANOSLEEP as usize] = SyscallEntry::args("nanosleep", syscall_nanosleep); // nanosleep(ms)

    table[abi::SYS_EXIT as usize] = SyscallEntry::frame("exit", syscall_exit); // exit(code)

    table
};

/// Entry from `isr_syscall`: runs the handler for `eax` and returns the ESP
/// to resume on (0 to return to the same task).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_dispatcher(regs: *mut ContextFrame) -> u32 {
    unsafe { sched::current().as_mut().unwrap() }.context.esp = regs as u32;

    let syscall_no = unsafe { (*regs).eax } as usize;
    if syscall_no < MAX_SYSCALL_NUMBER
        && let Some(entry) = &SYSCALL_ENTRIES[syscall_no]
    {
        let syscall_name = entry.name;

        unsafe {
            let result = entry.invoke(regs);

            if (*regs).is_error() {
                pr_debug!(
                    "Syscall {} returned error: {:?}",
                    syscall_name,
                    (*regs).get_error().unwrap_or(KernelError::TooBig)
                );
            } else {
                pr_debug!(
                    "Syscall {} returned value: {}",
                    syscall_name,
                    (*regs).get_return_value()
                );
            }
            if result != 0 {
                return result;
            }
            if sched::current().as_ref().unwrap().state == sched::ProcessState::Waiting {
                return sched::schedule(regs as u32);
            }
        }
        return 0;
    }

    pr_warn!("Unknown syscall number: {}", syscall_no);
    unsafe { (*regs).set_return_error(KernelError::NoSys) };
    0
}

#[unsafe(link_section = ".init.text")]
pub fn init_syscalls() {
    register_user_interrupt_handler(SYSCALL_VECTOR, isr_syscall);
}

crate::core_initcall!(init_syscalls);
