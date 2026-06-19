#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Ready,
    Running,
    Sleeping,
    Zombie,
    Thread,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Context {
    pub esp: u32, // kernel stack pointer
    pub cr3: u32, // physical address of page directory
}

#[derive(Debug, Clone, Copy)]
pub struct ProcessMemory {
    pub code_base: u32,
    pub code_size: u32,
    pub stack_base: u32,
    pub stack_limit: u32,
    pub heap_base: u32,
    pub heap_brk: u32,
}

pub const MAX_CHILDREN: usize = 16;
#[derive(Debug, Clone, Copy)]
pub struct ProcessFamily {
    pub parent_pid: u32,
    pub children: [u32; MAX_CHILDREN],
    pub child_count: usize,
}

pub const MAX_SIGNALS: usize = 32;
pub const SIGNAL_QUEUE_SIZE: usize = 16;
#[derive(Clone, Copy)]
pub struct SignalQueue {
    pub pending: [u8; SIGNAL_QUEUE_SIZE],
    pub head: usize,
    pub tail: usize,
    pub handlers: [u32; MAX_SIGNALS],
}

pub struct TaskStruct {
    pub pid: u32,
    pub uid: u32,
    pub state: ProcessState,
    pub context: Context,

    pub memory: ProcessMemory,
    pub family: ProcessFamily,
    pub signals: SignalQueue,

	pub kernel_stack_top: u32,
    pub kernel_stack_bottom: u32,
    pub tty_id: usize,
    pub wakeup_time: u64,
}

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct ContextFrame {
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
    pub const fn arg4(&self) -> u32 {
        self.esi
    }
    pub const fn arg5(&self) -> u32 {
        self.edi
    }
    pub const fn arg6(&self) -> u32 {
        self.ebp
    }
    pub fn set_return_value(&mut self, value: u32) {
        self.eax = value;
    }
}