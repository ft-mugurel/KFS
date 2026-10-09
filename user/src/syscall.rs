// =============================================================================
// KFS User-Space Syscall Interface
// =============================================================================

pub const SYS_EXIT: u32 = 1;
pub const SYS_FORK: u32 = 2;
pub const SYS_READ: u32 = 3;
pub const SYS_WRITE: u32 = 4;
pub const SYS_OPEN: u32 = 5;
pub const SYS_CLOSE: u32 = 6;
pub const SYS_WAITPID: u32 = 7;
pub const SYS_UNLINK: u32 = 10;
pub const SYS_CHDIR: u32 = 12;
pub const SYS_MKNOD: u32 = 14;
pub const SYS_GETPID: u32 = 20;
pub const SYS_KILL: u32 = 37;
pub const SYS_PIPE: u32 = 42;
pub const SYS_SBRK: u32 = 45;
pub const SYS_GETPPID: u32 = 64;
pub const SYS_MMAP: u32 = 90;
pub const SYS_MUNMAP: u32 = 91;
pub const SYS_SOCKET: u32 = 97;
pub const SYS_GETDENTS: u32 = 141;
pub const SYS_NANOSLEEP: u32 = 162;
pub const SYS_GETCWD: u32 = 183;
pub const SYS_GETUID: u32 = 199;
pub const SYS_LOGIN: u32 = 212;
pub const SYS_GETUSERNAME: u32 = 213;
pub const SYS_GETTTYNAME: u32 = 214;
pub const SYS_DEBUG: u32 = 223;

pub const O_RDONLY: u32 = 0;
pub const O_WRONLY: u32 = 1;
pub const O_RDWR: u32 = 2;
pub const O_CREAT: u32 = 0x40;
pub const O_TRUNC: u32 = 0x200;

#[repr(C, packed)]
pub struct LinuxDirent {
    pub d_ino: u32,
    pub d_off: u32,
    pub d_reclen: u16,
}

#[allow(dead_code)]
#[repr(u32)]
pub enum DebugOp {
    Memdump = 0,
    Pte = 1,
    Memtest = 2,
    Stack = 3,
    Layout = 4,
    Crash = 5,
    Loglevel = 6,
    Color = 7,
    Screen = 8,
    Reboot = 9,
    Shutdown = 10,
    Initcalls = 11,
    Clear = 12,
    Users = 13,
    UserAdd = 14,
    Su = 15,
    Ps = 16,
    Dmesg = 17,
    Klog = 18,
    ConsoleScreen = 19,
    UserDel = 20,
}

#[inline(always)]
pub unsafe fn syscall0(num: u32) -> i32 {
    let ret: i32;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inlateout("eax") num as i32 => ret,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
pub unsafe fn syscall1(num: u32, a1: u32) -> i32 {
    let ret: i32;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inlateout("eax") num as i32 => ret,
            in("ebx") a1,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
pub unsafe fn syscall2(num: u32, a1: u32, a2: u32) -> i32 {
    let ret: i32;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inlateout("eax") num as i32 => ret,
            in("ebx") a1,
            in("ecx") a2,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
pub unsafe fn syscall3(num: u32, a1: u32, a2: u32, a3: u32) -> i32 {
    let ret: i32;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inlateout("eax") num as i32 => ret,
            in("ebx") a1,
            in("ecx") a2,
            in("edx") a3,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
pub unsafe fn syscall4(num: u32, a1: u32, a2: u32, a3: u32, a4: u32) -> i32 {
    let ret: i32;
    unsafe {
        core::arch::asm!(
            "push esi",
            "mov esi, {a4}",
            "int 0x80",
            "pop esi",
            a4 = in(reg) a4,
            inlateout("eax") num as i32 => ret,
            in("ebx") a1,
            in("ecx") a2,
            in("edx") a3,
        );
    }
    ret
}

#[inline(always)]
pub unsafe fn syscall5(num: u32, a1: u32, a2: u32, a3: u32, a4: u32, a5: u32) -> i32 {
    let ret: i32;
    unsafe {
        core::arch::asm!(
            "push esi",
            "mov esi, {a4}",
            "int 0x80",
            "pop esi",
            a4 = in(reg) a4,
            inlateout("eax") num as i32 => ret,
            in("ebx") a1,
            in("ecx") a2,
            in("edx") a3,
            in("edi") a5,
        );
    }
    ret
}

pub fn sys_exit(code: i32) -> ! {
    unsafe {
        syscall1(SYS_EXIT, code as u32);
        loop {
            core::arch::asm!("hlt");
        }
    }
}

pub fn sys_fork() -> isize {
    unsafe { syscall0(SYS_FORK) as isize }
}

pub fn sys_read(fd: usize, buf: &mut [u8]) -> isize {
    unsafe { syscall3(SYS_READ, fd as u32, buf.as_mut_ptr() as u32, buf.len() as u32) as isize }
}

pub fn sys_write(fd: usize, buf: &[u8]) -> isize {
    unsafe { syscall3(SYS_WRITE, fd as u32, buf.as_ptr() as u32, buf.len() as u32) as isize }
}

pub fn sys_open(path: &[u8], flags: u32, mode: u32) -> isize {
    unsafe { syscall3(SYS_OPEN, path.as_ptr() as u32, flags, mode) as isize }
}

pub fn sys_close(fd: usize) -> isize {
    unsafe { syscall1(SYS_CLOSE, fd as u32) as isize }
}

pub fn sys_wait() -> isize {
    unsafe { syscall3(SYS_WAITPID, u32::MAX, 0, 0) as isize }
}

pub fn sys_kill(pid: usize, sig: u32) -> isize {
    unsafe { syscall2(SYS_KILL, pid as u32, sig) as isize }
}

pub fn sys_getcwd(buf: &mut [u8]) -> isize {
    unsafe { syscall2(SYS_GETCWD, buf.as_mut_ptr() as u32, buf.len() as u32) as isize }
}

pub fn sys_chdir(path: &[u8]) -> isize {
    unsafe { syscall1(SYS_CHDIR, path.as_ptr() as u32) as isize }
}

pub fn sys_getdents(fd: usize, buf: &mut [u8]) -> isize {
    unsafe { syscall3(SYS_GETDENTS, fd as u32, buf.as_mut_ptr() as u32, buf.len() as u32) as isize }
}

pub fn sys_mknod(path: &[u8], mode: u32) -> isize {
    unsafe { syscall3(SYS_MKNOD, path.as_ptr() as u32, mode, 0) as isize }
}

pub fn sys_getuid() -> u32 {
    unsafe { syscall0(SYS_GETUID) as u32 }
}

pub fn sys_login(username: &[u8], password: &[u8]) -> isize {
    unsafe {
        syscall4(
            SYS_LOGIN,
            username.as_ptr() as u32,
            username.len() as u32,
            password.as_ptr() as u32,
            password.len() as u32,
        ) as isize
    }
}

pub fn sys_getusername(buf: &mut [u8]) -> isize {
    unsafe { syscall2(SYS_GETUSERNAME, buf.as_mut_ptr() as u32, buf.len() as u32) as isize }
}

pub fn sys_getttyname(fd: usize, buf: &mut [u8]) -> isize {
    unsafe { syscall3(SYS_GETTTYNAME, fd as u32, buf.as_mut_ptr() as u32, buf.len() as u32) as isize }
}

pub fn sys_debug(op: DebugOp, a1: u32, a2: u32) -> isize {
    unsafe { syscall3(SYS_DEBUG, op as u32, a1, a2) as isize }
}

pub fn sys_debug4(op: DebugOp, a1: u32, a2: u32, a3: u32, a4: u32) -> isize {
    unsafe { syscall5(SYS_DEBUG, op as u32, a1, a2, a3, a4) as isize }
}

pub fn sys_getpid() -> usize {
    unsafe { syscall0(SYS_GETPID) as usize }
}

pub fn sys_getppid() -> usize {
    unsafe { syscall0(SYS_GETPPID) as usize }
}

pub fn sys_unlink(path: &[u8]) -> isize {
    unsafe { syscall1(SYS_UNLINK, path.as_ptr() as u32) as isize }
}

pub fn sys_pipe(fds: &mut [i32; 2]) -> isize {
    unsafe { syscall1(SYS_PIPE, fds.as_mut_ptr() as u32) as isize }
}

pub fn sys_sbrk(increment: i32) -> isize {
    unsafe { syscall1(SYS_SBRK, increment as u32) as isize }
}

pub fn sys_mmap(addr: u32, length: usize) -> isize {
    unsafe { syscall2(SYS_MMAP, addr, length as u32) as isize }
}

pub fn sys_munmap(addr: u32, length: usize) -> isize {
    unsafe { syscall2(SYS_MUNMAP, addr, length as u32) as isize }
}

pub fn sys_socket(domain: u32, sock_type: u32, protocol: u32) -> isize {
    unsafe { syscall3(SYS_SOCKET, domain, sock_type, protocol) as isize }
}

pub fn sys_nanosleep(ms: u32) -> isize {
    unsafe { syscall1(SYS_NANOSLEEP, ms) as isize }
}

pub fn sys_raw(num: u32, a1: u32, a2: u32, a3: u32) -> isize {
    unsafe { syscall3(num, a1, a2, a3) as isize }
}

