use core::{
    fmt::Display,
    mem::MaybeUninit,
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{fs::VfsNode, locks::Spinlock, x86};

mod process;
mod runqueue;
mod scheduler;
pub(crate) mod stats;
mod thread_info;

pub(crate) const THREAD_SIZE: usize = 16384;
pub(crate) const MAX_PROCESSES: usize = 64;
pub(crate) const MAX_CHILDREN: usize = 16;
pub(crate) const MAX_FDS_PER_PROCESS: usize = 64;
pub(crate) const MAX_SIGNALS: usize = 32;
pub(crate) const SIGNAL_QUEUE_SIZE: usize = 16;
pub(crate) const MAX_VMAS: usize = 16;
pub(crate) const VMA_READ: u32 = 1;
pub(crate) const VMA_WRITE: u32 = 2;
#[expect(dead_code, reason = "ELF loader will set it from PF_X")]
pub(crate) const VMA_EXEC: u32 = 4;
pub(crate) const TASK_COMM_LEN: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    pub env_start: u32,
    pub env_end: u32,
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

    pub fn set_handler(&mut self, sig: usize, handler: u32) {
        if sig < MAX_SIGNALS {
            self.handlers[sig] = handler;
        }
    }

    pub fn get_handler(&self, sig: usize) -> u32 {
        if sig < MAX_SIGNALS { self.handlers[sig] } else { 0 }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Credentials {
    pub uid: u32,
    pub gid: u32,
    pub euid: u32,
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
        self.fsgid == gid || self.groups[..group_count].contains(&gid)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TaskStruct {
    pub pid: u32,
    pub comm: [u8; TASK_COMM_LEN],
    pub credentials: Credentials,
    pub state: ProcessState,
    pub context: Context,

    pub memory: ProcessMemory,
    pub family: ProcessFamily,
    pub signals: SignalQueue,
    pub fd_tbl: [Option<usize>; MAX_FDS_PER_PROCESS],

    pub cwd: *mut VfsNode,
    pub env_start: u32,

    pub kernel_stack_top: u32,
    pub kernel_stack_bottom: u32,
    pub wakeup_time: u64,

    pub exit_code: Option<u32>,

    /// `fs::NodePin`s held, per grace bucket. Released if the task is killed.
    pub vfs_pins: [u32; 2],
}

impl TaskStruct {
    pub fn set_comm(&mut self, name: &[u8]) {
        let len = name.len().min(TASK_COMM_LEN - 1);
        self.comm = [0; TASK_COMM_LEN];
        self.comm[..len].copy_from_slice(&name[..len]);
    }

    pub fn comm(&self) -> &str {
        let len = self
            .comm
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(TASK_COMM_LEN);
        core::str::from_utf8(&self.comm[..len]).unwrap_or("?")
    }

    pub fn alloc_fd(&mut self) -> Option<usize> {
        (3..MAX_FDS_PER_PROCESS).find(|&i| self.fd_tbl[i].is_none())
    }
}

unsafe impl Send for TaskStruct {}

const EMPTY_VMA: Vma = Vma { base: 0, size: 0, flags: 0, used: false };

pub(crate) static PROCESS_TABLE: Spinlock<[Option<TaskStruct>; MAX_PROCESSES]> = {
    const EMPTY: Option<TaskStruct> = None;
    Spinlock::new([EMPTY; MAX_PROCESSES])
};

pub(crate) fn set_state(task: &mut TaskStruct, state: ProcessState) {
    let old_state = task.state;
    if old_state == state {
        return;
    }
    task.state = state;
    runqueue::state_changed(task.pid as usize, old_state, state, task.wakeup_time);
}

pub(crate) fn runnable_task_count() -> usize {
    runqueue::runnable_count()
}

pub(crate) fn has_runnable_tasks() -> bool {
    runnable_task_count() != 0
}

pub(crate) fn wakeup_may_be_due(current_tick: u64) -> bool {
    runqueue::wakeup_may_be_due(current_tick)
}

/// Frees orphaned zombies (whose parent is an idle task): their table slot,
/// procfs entry, address space and kernel stack.
///
/// # Safety
/// Must not hold PROCESS_TABLE. A zombie must no longer be running on any
/// CPU, because its kernel stack and page directory are freed.
pub unsafe fn reap_zombies() {
    let mut to_reap = [None; MAX_PROCESSES];
    let mut count = 0;
    {
        let mut table = PROCESS_TABLE.lock();
        for pid in crate::smp::MAX_CPUS..MAX_PROCESSES {
            if let Some(ref task) = table[pid]
                && task.state == ProcessState::Zombie
                && (task.family.parent_pid as usize) < crate::smp::MAX_CPUS
            {
                to_reap[count] = Some((pid, task.pid, task.context.cr3, task.kernel_stack_bottom));
                count += 1;
                table[pid] = None;
            }
        }
        if count > 0
            && let Some(ref mut init_task) = table[0]
        {
            let mut new_children = [0u32; MAX_CHILDREN];
            let mut new_count = 0;
            for cpid in init_task
                .family
                .children
                .iter()
                .take(init_task.family.child_count)
            {
                let reaped = to_reap
                    .iter()
                    .take(count)
                    .flatten()
                    .any(|(_, rpid, _, _)| *rpid == *cpid);
                if !reaped && new_count < MAX_CHILDREN {
                    new_children[new_count] = *cpid;
                    new_count += 1;
                }
            }
            init_task.family.children = new_children;
            init_task.family.child_count = new_count;
        }
    }

    for item in to_reap.iter().take(count) {
        if let Some((_, rpid, old_cr3, k_stack_bottom)) = *item {
            // SAFETY: the task is gone from PROCESS_TABLE and no longer runs,
            // so nothing uses its procfs nodes, page directory or kernel stack.
            // The kernel stack is in the kernel half, mapped in every directory.
            unsafe {
                crate::fs::procfs::process_reaped(rpid);
                crate::paging::free_user_address_space(old_cr3);
            }
            if let Some(k_stack_phys) = unsafe { crate::paging::virt_to_phys(k_stack_bottom) } {
                let frames_needed = THREAD_SIZE / crate::paging::PAGE_SIZE;
                let _ = crate::paging::free_contiguous_physical_pages(k_stack_phys, frames_needed);
            }
        }
    }
}

pub(crate) fn reserve_process_slot() -> Option<usize> {
    unsafe {
        reap_zombies();
    }
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

pub(crate) use process::spawn_init_shells;
pub(crate) use scheduler::{
    idle_stack_top, init_boot_thread_info, init_scheduler_for_cpu, schedule,
};
pub(crate) use thread_info::{
    ContextFrame, STACK_CANARY, ThreadInfo, canary_error_count, current, current_cpu, current_cred,
    current_pid, print_errors,
};

pub(crate) fn runqueue_selftest(check: &mut dyn FnMut(&'static str, bool)) {
    runqueue::selftest(check);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn idle_loop() -> ! {
    // APs reach here before /dev exists; their idle tasks never do I/O and
    // kernel-spawned processes don't inherit idle descriptors.
    if !crate::fs::root_filesystem_ready() {
    } else if let Err(error) = unsafe { crate::tty::bind_stdio() } {
        crate::pr_err!("Failed to bind standard terminal descriptors: {:?}", error);
    }
    unsafe { x86::enable_interrupts() };
    loop {
        unsafe { reap_zombies() };
        x86::hlt();
    }
}

static SCHEDULER_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn is_scheduler_active() -> bool {
    SCHEDULER_ACTIVE.load(Ordering::SeqCst)
}

/// Moves esp onto the given stack and jumps (not calls) into idle_loop.
/// Never returns, there is no valid frame to return to on the old stack.
///
/// # Safety
/// `new_esp` must be the top of this CPU's own idle stack (`idle_stack_top`),
/// with its ThreadInfo set up by `init_scheduler_for_cpu`. Anything still on
/// the current stack is abandoned.
pub unsafe fn switch_to_idle_stack(new_esp: u32) -> ! {
    SCHEDULER_ACTIVE.store(true, Ordering::SeqCst);
    unsafe {
        core::arch::asm!(
            "mov esp, {esp}",
            "jmp {func}",
            esp = in(reg) new_esp,
            func = sym idle_loop,
            options(noreturn)
        );
    }
}
