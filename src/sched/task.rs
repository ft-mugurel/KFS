#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Ready,
    Running,
    Sleeping,
    Zombie,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Context {
    pub esp: u32, // kernel stack pointer
    pub cr3: u32, // physical address of page directory
}

impl Context {
    pub const fn new() -> Self {
        Self { esp: 0, cr3: 0 }
    }
}

pub struct TaskStruct {
    pub pid: u32,
    pub parent_pid: u32,
    pub state: ProcessState,
    pub context: Context,
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