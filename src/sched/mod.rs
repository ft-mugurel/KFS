use core::{fmt::Display, mem::MaybeUninit};

use crate::{fs::VfsNode, locks::Spinlock, x86};

mod process;
mod scheduler;
mod thread_info;
// mod task_queue;

pub(crate) const THREAD_SIZE: usize = 16384;
pub(crate) const MAX_PROCESSES: usize = 64;
pub(crate) const MAX_CHILDREN: usize = 16;
pub(crate) const MAX_FDS_PER_PROCESS: usize = 16;
pub(crate) const MAX_SIGNALS: usize = 32;
pub(crate) const SIGNAL_QUEUE_SIZE: usize = 16;
pub(crate) const MAX_VMAS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ProcessState {
    Ready,
    Running,
    Sleeping,
    Waiting,
    Zombie,
    Terminated,
}

impl Display for ProcessState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let state_str = match self {
            ProcessState::Ready => "Ready",
            ProcessState::Running => "Running",
            ProcessState::Sleeping => "Sleeping",
            ProcessState::Waiting => "Waiting",
            ProcessState::Zombie => "Zombie",
            ProcessState::Terminated => "Terminated",
        };
        write!(f, "{}", state_str)
    }
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
    #[allow(dead_code)]
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

impl SignalQueue {
    #[allow(dead_code)]
    pub const fn new() -> Self {
        Self {
            pending: [0; SIGNAL_QUEUE_SIZE],
            head: 0,
            tail: 0,
            handlers: [0; MAX_SIGNALS],
        }
    }

    pub fn push(&mut self, sig: u8) -> bool {
        let next_tail = (self.tail + 1) % SIGNAL_QUEUE_SIZE;
        if next_tail != self.head {
            self.pending[self.tail] = sig;
            self.tail = next_tail;
            true
        } else {
            false
        }
    }

    pub fn pop(&mut self) -> Option<u8> {
        if self.head != self.tail {
            let sig = self.pending[self.head];
            self.head = (self.head + 1) % SIGNAL_QUEUE_SIZE;
            Some(sig)
        } else {
            None
        }
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    pub fn set_handler(&mut self, sig: usize, handler: u32) {
        if sig < MAX_SIGNALS {
            self.handlers[sig] = handler;
        }
    }

    pub fn get_handler(&self, sig: usize) -> u32 {
        if sig < MAX_SIGNALS {
            self.handlers[sig]
        } else {
            0
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Credentials {
    pub uid: u32,
    pub gid: u32,
    pub euid: u32,
    #[allow(dead_code)]
    pub egid: u32,
    pub fsuid: u32,
    pub fsgid: u32,
    pub groups: [u32; 8],
    pub group_count: u8,
}

impl Credentials {
    pub const fn root() -> Self {
        Self {
            uid: 0,
            gid: 0,
            euid: 0,
            egid: 0,
            fsuid: 0,
            fsgid: 0,
            groups: [0; 8],
            group_count: 0,
        }
    }

    pub fn is_root(&self) -> bool {
        self.euid == 0
    }

    pub fn in_group(&self, gid: u32) -> bool {
        let group_count = (self.group_count as usize).min(self.groups.len());
        self.fsgid == gid || self.groups[..group_count].iter().any(|&group| group == gid)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TaskStruct {
    pub pid: u32,
    pub credentials: Credentials,
    pub state: ProcessState,
    pub context: Context,

    pub memory: ProcessMemory,
    pub family: ProcessFamily,
    pub signals: SignalQueue,
    pub fd_tbl: [Option<usize>; MAX_FDS_PER_PROCESS],

    pub cwd: *mut VfsNode,

    pub kernel_stack_top: u32,
    pub kernel_stack_bottom: u32,
    pub wakeup_time: u64,

    pub exit_code: Option<u32>,
}

impl TaskStruct {
    pub fn alloc_fd(&mut self) -> Option<usize> {
        for i in 3..MAX_FDS_PER_PROCESS {
            if self.fd_tbl[i].is_none() {
                return Some(i);
            }
        }
        None
    }
}

unsafe impl Send for TaskStruct {}

const EMPTY_VMA: Vma = Vma { base: 0, size: 0, flags: 0, used: false };

pub(crate) static PROCESS_TABLE: Spinlock<[Option<TaskStruct>; MAX_PROCESSES]> = {
    const EMPTY: Option<TaskStruct> = None;
    Spinlock::new([EMPTY; MAX_PROCESSES])
};

pub(crate) fn reserve_process_slot() -> Option<usize> {
    let mut table = PROCESS_TABLE.lock();
    for index in crate::smp::MAX_CPUS..MAX_PROCESSES {
        if table[index].is_none() {
            let mut new_task: TaskStruct = unsafe { MaybeUninit::zeroed().assume_init() };
            new_task.state = ProcessState::Terminated;
            table[index] = Some(new_task);
            return Some(index);
        }
    }
    None
}

pub(crate) use process::create_user_process;
pub(crate) use scheduler::{idle_stack_top, init_scheduler_for_cpu, schedule};
pub(crate) use thread_info::{
    current, current_cpu, current_cred, current_pid, ContextFrame, ThreadInfo, STACK_CANARY,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn idle_loop() -> ! {
    if let Err(error) = crate::tty::bind_stdio() {
        crate::pr_err!(
            "Failed to bind standard terminal descriptors: {:?}\n",
            error
        );
    }
    x86::enable_interrupts();
    loop {
        x86::hlt();
    }
}

/// Moves esp onto the given stack and jumps (not calls) into idle_loop.
/// Never returns — there is no valid frame to return to on the old stack.
pub unsafe fn switch_to_idle_stack(new_esp: u32) -> ! {
    core::arch::asm!(
        "mov esp, {esp}",
        "jmp {func}",
        esp = in(reg) new_esp,
        func = sym idle_loop,
        options(noreturn)
    );
}
