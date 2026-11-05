//! Typed syscall arguments.
//!
//! The dispatcher copies the six argument registers out of the frame once
//! (`SyscallArgs`), and `syscall_define!` turns them into typed parameters.
//! Handlers written this way never see the frame: they return
//! `KResult<u32>`, and the dispatcher writes `eax` (or rewinds `eip` for
//! `Err(KernelError::Restart)`).
//!
//! The set of argument types stays small. It describes how a register is
//! interpreted, not what it points to:
//! - integers: `u32`, `i32`, `usize`
//! - `UserAddr`: a user address that is never dereferenced (mmap hint, handler)
//! - `UserPtr<T>`: a user pointer read or written as a `T: UserCopy`
//! - `UserSlice`: a user buffer, built in the handler from an address and a length
//!
//! There is deliberately no `u8`/`u16` conversion: truncating a register
//! silently turns out-of-range values into valid ones (`kill(pid, 265)` would
//! become signal 9). Take a `u32` and range-check it instead.

use core::marker::PhantomData;

use super::read_write::valid_user_buffer;
use crate::error::{KResult, KernelError};
use crate::fs::VfsNode;
use crate::sched::{self, ContextFrame, Credentials};

pub const PATH_MAX: usize = 1024;

#[derive(Clone, Copy)]
pub struct SyscallArgs([u32; 6]);

impl SyscallArgs {
    pub(super) fn from_frame(frame: &ContextFrame) -> Self {
        Self([frame.ebx, frame.ecx, frame.edx, frame.esi, frame.edi, frame.ebp])
    }

    pub fn get<T: FromSyscallArg>(&self, index: usize) -> T {
        T::from_arg(self.0[index])
    }
}

pub trait FromSyscallArg {
    fn from_arg(raw: u32) -> Self;
}

impl FromSyscallArg for u32 {
    fn from_arg(raw: u32) -> Self {
        raw
    }
}

impl FromSyscallArg for i32 {
    fn from_arg(raw: u32) -> Self {
        raw as i32
    }
}

impl FromSyscallArg for usize {
    fn from_arg(raw: u32) -> Self {
        raw as usize
    }
}

/// A user-space address that the kernel stores or compares but never
/// dereferences (mmap hints, signal handler entry points).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserAddr(pub u32);

impl FromSyscallArg for UserAddr {
    fn from_arg(raw: u32) -> Self {
        UserAddr(raw)
    }
}

/// Types that may be copied to or from user memory byte for byte.
///
/// # Safety
/// The type must be `#[repr(C)]` (or a primitive/array of them) and every bit
/// pattern must be a valid value: no references, `bool`s, enums, or padding
/// the kernel relies on. A user process can write any bytes into its memory.
pub unsafe trait UserCopy: Copy {}

macro_rules! user_copy_primitives {
    ($($t:ty),*) => { $(
        unsafe impl UserCopy for $t {}
    )* };
}
user_copy_primitives!(u8, u16, u32, u64, i8, i16, i32, i64, usize, isize);

unsafe impl<T: UserCopy, const N: usize> UserCopy for [T; N] {}
// All `u32` fields: no padding, and any bytes are valid.
unsafe impl UserCopy for kfs_abi::Stat {}

/// A pointer into the calling process's memory. Holding one doesn't mean the
/// address is valid: `read` and `write` check the range first.
pub struct UserPtr<T> {
    addr: u32,
    _t: PhantomData<*mut T>,
}

// Manual impls: `derive` would wrongly require `T: Clone`/`T: Copy`.
impl<T> Clone for UserPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for UserPtr<T> {}

impl<T> FromSyscallArg for UserPtr<T> {
    fn from_arg(raw: u32) -> Self {
        UserPtr::new(raw)
    }
}

impl<T> UserPtr<T> {
    pub const fn new(addr: u32) -> Self {
        UserPtr { addr, _t: PhantomData }
    }

    #[expect(dead_code, reason = "handlers not converted to UserPtr yet")]
    pub const fn addr(self) -> u32 {
        self.addr
    }

    pub const fn is_null(self) -> bool {
        self.addr == 0
    }

    #[expect(dead_code, reason = "handlers not converted to UserPtr yet")]
    pub const fn cast<U>(self) -> UserPtr<U> {
        UserPtr::new(self.addr)
    }

    pub fn validate(self) -> KResult<()> {
        if self.is_null() || !unsafe { valid_user_buffer(self.addr, size_of::<T>()) } {
            return Err(KernelError::Fault);
        }
        Ok(())
    }

    pub fn read(self) -> KResult<T>
    where
        T: UserCopy,
    {
        self.validate()?;
        // SAFETY: the whole range is the caller's mapped memory (checked
        // above), `T: UserCopy` accepts any bytes, and the read is unaligned
        // because user pointers carry no alignment guarantee.
        Ok(unsafe { (self.addr as *const T).read_unaligned() })
    }

    pub fn write(self, value: &T) -> KResult<()>
    where
        T: UserCopy,
    {
        self.validate()?;
        unsafe { (self.addr as *mut T).write_unaligned(*value) };
        Ok(())
    }
}

impl UserPtr<u8> {
    /// Reads a NUL-terminated UTF-8 string of at most `max_len` bytes
    /// (excluding the NUL), checking each byte is in the caller's memory.
    pub fn read_cstr<'a>(self, max_len: usize) -> KResult<&'a str> {
        if self.is_null() {
            return Err(KernelError::Fault);
        }
        let mut len = 0;
        loop {
            let byte = UserPtr::<u8>::new(self.addr.checked_add(len as u32).ok_or(KernelError::Fault)?)
                .read()?;
            if byte == 0 {
                break;
            }
            len += 1;
            if len > max_len {
                return Err(KernelError::NameTooLong);
            }
        }
        let bytes = unsafe { core::slice::from_raw_parts(self.addr as *const u8, len) };
        core::str::from_utf8(bytes).map_err(|_| KernelError::Inval)
    }
}

/// A user buffer, validated when it is created.
///
/// The caller's memory can only change while this process runs, and it is
/// blocked in the syscall, so handing the VFS a slice is sound. Don't create
/// two overlapping `UserSlice`s in one syscall and use both mutably.
pub struct UserSlice {
    addr: u32,
    len: usize,
}

impl UserSlice {
    pub fn new(addr: UserAddr, len: usize) -> KResult<Self> {
        if !unsafe { valid_user_buffer(addr.0, len) } || (addr.0 == 0 && len != 0) {
            return Err(KernelError::Fault);
        }
        Ok(UserSlice { addr: addr.0, len })
    }

    #[expect(dead_code, reason = "handlers not converted to UserPtr yet")]
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn as_bytes(&self) -> &[u8] {
        if self.len == 0 {
            return &[];
        }
        unsafe { core::slice::from_raw_parts(self.addr as *const u8, self.len) }
    }

    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        if self.len == 0 {
            return &mut [];
        }
        unsafe { core::slice::from_raw_parts_mut(self.addr as *mut u8, self.len) }
    }
}

/// Defines a syscall handler with typed parameters, like Linux's
/// `SYSCALL_DEFINEn`. Each parameter takes the next argument register through
/// `FromSyscallArg`; more than six parameters is a compile error.
///
/// ```ignore
/// syscall_define! {
///     pub(super) fn syscall_kill(pid: usize, sig: u32) {
///         ...
///         Ok(0)
///     }
/// }
/// ```
/// expands to `fn syscall_kill(args: SyscallArgs) -> KResult<u32>`, the shape
/// the dispatcher's `SyscallEntry::Args` expects.
macro_rules! syscall_define {
    (@call $f:ident, $args:ident, [$($idx:tt)*], [$($done:expr),*], ) => {
        $f($($done),*)
    };
    (@call $f:ident, $args:ident, [$i:tt $($rest:tt)*], [$($done:expr),*], $ty:ty $(, $more:ty)*) => {
        syscall_define!(@call $f, $args, [$($rest)*], [$($done,)* $args.get::<$ty>($i)], $($more),*)
    };
    (@call $f:ident, $args:ident, [], [$($done:expr),*], $ty:ty $(, $more:ty)*) => {
        compile_error!("a syscall takes at most 6 arguments")
    };
    ($(#[$meta:meta])* $vis:vis fn $name:ident($($arg:ident: $ty:ty),* $(,)?) $body:block) => {
        $(#[$meta])*
        #[allow(unused_variables)] // `args` is unused when there are no parameters
        $vis fn $name(args: $crate::syscalls::SyscallArgs) -> $crate::error::KResult<u32> {
            #[inline(always)]
            fn body($($arg: $ty),*) -> $crate::error::KResult<u32> $body
            syscall_define!(@call body, args, [0 1 2 3 4 5], [], $($ty),*)
        }
    };
}
pub(crate) use syscall_define;

/// What path-based syscalls need from the calling task, copied out so no
/// reference to the task stays alive across VFS calls (which can block or
/// take other locks).
#[derive(Clone, Copy)]
pub(crate) struct Caller {
    // This is safe, trust me
    pub cwd: *mut VfsNode,
    pub credentials: Credentials,
}

pub(crate) fn caller() -> KResult<Caller> {
    let task = unsafe { sched::current().as_ref() }.ok_or(KernelError::Srch)?;
    Ok(Caller { cwd: task.cwd, credentials: task.credentials })
}