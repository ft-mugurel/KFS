use super::TaskStruct;
use crate::error::KernelError;
use crate::sched::THREAD_SIZE;

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
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

impl ContextFrame {
    pub const fn arg1(&self) -> u32 {
        self.ebx
    }
    pub const fn arg2(&self) -> u32 {
        self.ecx
    }
    pub const fn arg3(&self) -> u32 {
        self.edx
    }
    #[allow(dead_code)]
    pub const fn arg4(&self) -> u32 {
        self.esi
    }
    #[allow(dead_code)]
    pub const fn arg5(&self) -> u32 {
        self.edi
    }
    #[allow(dead_code)]
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
    pub fn get_error(&self) -> Option<KernelError> {
        if self.is_error() {
            Some(unsafe { core::mem::transmute(self.eax) })
        } else {
            None
        }
    }
}

pub(super) const STACK_CANARY: u32 = 0xDEAD_C0DE;

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
pub unsafe fn current_thread_info() -> *mut ThreadInfo {
    let esp: u32;
    core::arch::asm!(
        "mov {}, esp",
        out(reg) esp,
        options(nomem, nostack, preserves_flags)
    );
    (esp & !(THREAD_SIZE as u32 - 1)) as *mut ThreadInfo
}

#[inline(always)]
pub unsafe fn current_pid() -> u32 {
    let ti = current_thread_info();
    if (*ti).canary != STACK_CANARY {
        return 0;
    }
    (*ti).task_pid
}

#[inline(always)]
pub unsafe fn current() -> *mut TaskStruct {
    (*current_thread_info()).task
}

#[inline(always)]
pub unsafe fn current_cpu() -> u32 {
    (*current_thread_info()).cpu_id
}
