use crate::fs::FileDescriptor;

mod scheduler;
mod task;
mod task_queue;
mod user_process;

pub(crate) const MAX_PROCESSES: usize = 64;
pub(crate) const MAX_CHILDREN: usize = 16;
pub(crate) const MAX_FDS_PER_PROCESS: usize = 16;
pub(crate) const MAX_SIGNALS: usize = 32;
pub(crate) const SIGNAL_QUEUE_SIZE: usize = 16;
pub(crate) const MAX_VMAS: usize = 16;

#[derive(PartialEq, Eq)]
pub(crate) enum ProcessState {
    Ready,
    Running,
    Sleeping,
    Zombie,
    Thread,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub(crate) struct Context {
    pub esp: u32, // kernel stack pointer
    pub cr3: u32, // physical address of page directory
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Vma {
    pub base: u32,
    pub size: u32,
    pub flags: u32,
    pub used: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProcessMemory {
    pub code_base: u32,
    pub code_size: u32,
    pub data_base: u32,
    pub data_size: u32,
    pub bss_base: u32,
    pub bss_size: u32,
    pub stack_base: u32,
    pub stack_limit: u32,
    pub heap_base: u32,
    pub heap_brk: u32,
    pub vmas: [Vma; MAX_VMAS],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProcessFamily {
    pub parent_pid: u32,
    pub children: [u32; MAX_CHILDREN],
    pub child_count: usize,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SignalQueue {
    pub pending: [u8; SIGNAL_QUEUE_SIZE],
    pub head: usize,
    pub tail: usize,
    pub handlers: [u32; MAX_SIGNALS],
}

pub(crate) struct TaskStruct {
    pub pid: u32,
    pub uid: u32,
    pub state: ProcessState,
    pub context: Context,

    pub memory: ProcessMemory,
    pub family: ProcessFamily,
    pub signals: SignalQueue,
    pub fd_tbl: [Option<FileDescriptor>; MAX_FDS_PER_PROCESS],

    pub kernel_stack_top: u32,
    pub kernel_stack_bottom: u32,
    pub wakeup_time: u64,

    pub exit_code: Option<u32>,
}

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

const EMPTY_VMA: Vma = Vma { base: 0, size: 0, flags: 0, used: false };

pub(crate) static mut PROCESS_TABLE: [Option<TaskStruct>; MAX_PROCESSES] = {
    const EMPTY: Option<TaskStruct> = None;
    [EMPTY; MAX_PROCESSES]
};
pub(crate) static mut CURRENT_PID: usize = 0;

pub(crate) use scheduler::{init_scheduler, schedule, yield_cpu};
pub(crate) use task_queue::{execute_tasks, schedule_task};
pub(crate) use user_process::create_user_process;
