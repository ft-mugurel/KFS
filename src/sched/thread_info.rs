use super::{Credentials, TaskStruct};
use crate::error::KernelError;
use crate::pr_err;
use crate::sched::THREAD_SIZE;

use core::sync::atomic::{AtomicU16, Ordering};

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub(crate) struct ContextFrame {
    pub edi: u32,
    pub esi: u32,
    pub ebp: u32,
    pub esp_dummy: u32,
    pub ebx: u32,
    pub edx: u32,
    pub ecx: u32,
    pub eax: u32,

    pub gs: u32,
    pub fs: u32,
    pub es: u32,
    pub ds: u32,

    pub eip: u32,
    pub cs: u32,
    pub eflags: u32,
    pub user_esp: u32,
    pub user_ss: u32,
}

// The ISR stubs push this layout by hand.
const _: () = assert!(size_of::<ContextFrame>() == 68);

impl ContextFrame {
    pub const fn arg1(&self) -> u32 {
        self.ebx
    }
    #[expect(dead_code, reason = "for syscalls with more arguments")]
    pub const fn arg2(&self) -> u32 {
        self.ecx
    }
    #[expect(dead_code, reason = "for syscalls with more arguments")]
    pub const fn arg3(&self) -> u32 {
        self.edx
    }
    #[expect(dead_code, reason = "for syscalls with more arguments")]
    pub const fn arg4(&self) -> u32 {
        self.esi
    }
    #[expect(dead_code, reason = "for syscalls with more arguments")]
    pub const fn arg5(&self) -> u32 {
        self.edi
    }
    #[expect(dead_code, reason = "for syscalls with more arguments")]
    pub const fn arg6(&self) -> u32 {
        self.ebp
    }
    pub fn set_return_value(&mut self, value: u32) {
        self.eax = value;
    }
    pub fn set_return_error(&mut self, error: KernelError) {
        self.eax = error as u32;
    }
    pub fn get_return_value(&self) -> u32 {
        self.eax
    }
    pub fn is_error(&self) -> bool {
        self.eax >= 0x80000000
    }
    /// The error in eax, if it holds one. A large non-error value (for
    /// example an address above 2 GiB) maps to `EIO` instead of being
    /// transmuted into an invalid `KernelError`.
    pub fn get_error(&self) -> Option<KernelError> {
        self.is_error()
            .then(|| KernelError::from_code(self.eax as i32))
    }
}

pub(crate) const STACK_CANARY: u32 = 0xDEAD_C0DE;
pub static PID_CANARY_ERRORS: AtomicU16 = AtomicU16::new(0);
pub static CPU_CANARY_ERRORS: AtomicU16 = AtomicU16::new(0);
pub static TASK_CANARY_ERRORS: AtomicU16 = AtomicU16::new(0);

#[repr(C, packed)]
pub struct ThreadInfo {
    pub task: *mut TaskStruct,
    pub task_pid: u32,
    pub cpu_id: u32,
    pub flags: u32,
    pub preempt_count: u32,
    pub canary: u32,
}

#[inline(always)]
pub fn current_thread_info() -> *mut ThreadInfo {
    let esp: u32;
    unsafe {
        core::arch::asm!(
            "mov {}, esp",
            out(reg) esp,
            options(nomem, nostack, preserves_flags)
        );
    }
    (esp & !(THREAD_SIZE as u32 - 1)) as *mut ThreadInfo
}

#[inline(always)]
pub unsafe fn current_pid() -> u32 {
    let ti = current_thread_info();
    if unsafe { (*ti).canary } != STACK_CANARY {
        PID_CANARY_ERRORS.fetch_add(1, Ordering::Relaxed);
        return 0;
    }
    let pid = unsafe { (*ti).task_pid };
    if (pid as usize) >= crate::sched::MAX_PROCESSES {
        // pr_err!("pid out of bounds, returning 0");
        return 0;
    }
    pid
}

#[inline(always)]
pub unsafe fn current() -> *mut TaskStruct {
    let ti = current_thread_info();
    if unsafe { (*ti).canary } != STACK_CANARY {
        TASK_CANARY_ERRORS.fetch_add(1, Ordering::Relaxed);
        return core::ptr::null_mut();
    }
    unsafe { (*ti).task }
}

#[inline(always)]
pub unsafe fn current_cpu() -> u32 {
    let ti = current_thread_info();
    if unsafe { (*ti).canary } != STACK_CANARY {
        CPU_CANARY_ERRORS.fetch_add(1, Ordering::Relaxed);
        return 0;
    }
    let cpu = unsafe { (*ti).cpu_id };
    if (cpu as usize) >= crate::smp::MAX_CPUS {
        // pr_err!("cpu out of bounds, returning 0");
        return 0;
    }
    cpu
}

pub fn canary_error_count() -> u32 {
    PID_CANARY_ERRORS.load(Ordering::Relaxed) as u32
        + CPU_CANARY_ERRORS.load(Ordering::Relaxed) as u32
        + TASK_CANARY_ERRORS.load(Ordering::Relaxed) as u32
}

pub unsafe fn print_errors() {
    let pid_errors = PID_CANARY_ERRORS.load(Ordering::Relaxed);
    let cpu_errors = CPU_CANARY_ERRORS.load(Ordering::Relaxed);
    let task_errors = TASK_CANARY_ERRORS.load(Ordering::Relaxed);
    if pid_errors > 0 || cpu_errors > 0 || task_errors > 0 {
        pr_err!(
            "ThreadInfo canary errors: PID={}, CPU={}, TASK={}",
            pid_errors,
            cpu_errors,
            task_errors
        );
    }
}

pub unsafe fn current_cred() -> *mut Credentials {
    let task = unsafe { current() };
    if task.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: `task` is the live current task (non-null, checked above). A
    // place projection, not `*task` as a value: TaskStruct is `Copy`, so
    // `unsafe { *task }` would point the result at a temporary copy.
    unsafe { &raw mut (*task).credentials }
}
