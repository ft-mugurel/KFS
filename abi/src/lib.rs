#![no_std]

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
pub const SYS_MOUNT: u32 = 21;
pub const SYS_KILL: u32 = 37;
pub const SYS_PIPE: u32 = 42;
pub const SYS_SBRK: u32 = 45;
pub const SYS_SIGNAL: u32 = 48;
pub const SYS_UMOUNT: u32 = 52;
pub const SYS_DUP2: u32 = 63;
pub const SYS_GETPPID: u32 = 64;
pub const SYS_READLINK: u32 = 85;
pub const SYS_MMAP: u32 = 90;
pub const SYS_MUNMAP: u32 = 91;
pub const SYS_SOCKET: u32 = 97;
pub const SYS_LSTAT: u32 = 107;
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

pub const S_IFMT: u32 = 0xF000;
pub const S_IFIFO: u32 = 0x1000;
pub const S_IFCHR: u32 = 0x2000;
pub const S_IFDIR: u32 = 0x4000;
pub const S_IFBLK: u32 = 0x6000;
pub const S_IFREG: u32 = 0x8000;
pub const S_IFLNK: u32 = 0xA000;
pub const S_IFSOCK: u32 = 0xC000;

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct Stat {
    pub st_ino: u32,
    pub st_mode: u32,
    pub st_nlink: u32,
    pub st_uid: u32,
    pub st_gid: u32,
    pub st_size: u32,
}

#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DebugOp {
    MemDump = 0,  // arg1: vaddr, arg2: len
    Pte = 1,      // arg1: vaddr
    MemTest = 2,  // arg1: test_ptr, arg2: test_len (or 0 for default)
    Stack = 3,    // arg1: words
    Layout = 4,   // display kernel segment layout
    Crash = 5,    // intentional test crash
    LogLevel = 6, // arg1: level (0..7)
    Color = 7,    // arg1: fg (0..15), arg2: bg (0..15)
    Screen = 8,   // arg1: screen index (0..5)
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
    PrintErrors = 22,
}

impl DebugOp {
    pub fn from_u32(val: u32) -> Option<Self> {
        match val {
            0 => Some(Self::MemDump),
            1 => Some(Self::Pte),
            2 => Some(Self::MemTest),
            3 => Some(Self::Stack),
            4 => Some(Self::Layout),
            5 => Some(Self::Crash),
            6 => Some(Self::LogLevel),
            7 => Some(Self::Color),
            8 => Some(Self::Screen),
            9 => Some(Self::Reboot),
            10 => Some(Self::Shutdown),
            11 => Some(Self::Initcalls),
            12 => Some(Self::Clear),
            13 => Some(Self::Users),
            14 => Some(Self::UserAdd),
            15 => Some(Self::Su),
            16 => Some(Self::Ps),
            17 => Some(Self::Dmesg),
            18 => Some(Self::Klog),
            19 => Some(Self::ConsoleScreen),
            20 => Some(Self::UserDel),
            22 => Some(Self::PrintErrors),
            _ => None,
        }
    }
}
