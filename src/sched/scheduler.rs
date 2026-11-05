use super::runqueue;
use super::{
    ContextFrame, Credentials, MAX_CHILDREN, MAX_FDS_PER_PROCESS, MAX_PROCESSES, PROCESS_TABLE,
    ProcessState, THREAD_SIZE, TaskStruct,
};
use crate::gdt;
use crate::interrupts::timer;
use crate::paging;
use crate::panic;
use crate::pr_debug;
use crate::pr_emerg;
use crate::pr_info;
use crate::sched::thread_info::STACK_CANARY;
use crate::sched::thread_info::{self, ThreadInfo};
use crate::smp::MAX_CPUS;
use crate::x86;

// One dedicated kernel stack + idle TaskStruct per core. PIDs 0..MAX_CPUS
// are reserved for idle tasks; real processes start at MAX_CPUS.
#[repr(C, align(4096))]
struct GuardPage([u8; 4096]);

#[unsafe(no_mangle)]
static mut IDLE_STACK_GUARD: GuardPage = GuardPage([0; 4096]);

#[repr(C, align(16384))]
struct IdleStack([u8; THREAD_SIZE]);

#[unsafe(no_mangle)]
static mut IDLE_KERNEL_STACKS: [IdleStack; MAX_CPUS] =
    [const { IdleStack([0; THREAD_SIZE]) }; MAX_CPUS];

pub fn idle_stack_top(cpu_id: usize) -> u32 {
    let k_stack_bottom = (unsafe { &raw const IDLE_KERNEL_STACKS[cpu_id] }) as u32;
    let k_stack_top = k_stack_bottom + THREAD_SIZE as u32;
    debug_assert_eq!(k_stack_bottom % THREAD_SIZE as u32, 0); // catch this class of bug immediately if it regresses
    k_stack_top
}

/// Minimal ThreadInfo for the BSP, which header.asm starts directly on
/// IDLE_KERNEL_STACKS[0]. Until init_scheduler_for_cpu(0) attaches the idle
/// task, lookups report cpu 0 / pid 0 / no task instead of failing the canary.
/// Must run before anything calls current*(), so no logging in here.
#[unsafe(link_section = ".init.text")]
pub fn init_boot_thread_info() {
    unsafe {
        let ti = (&raw mut IDLE_KERNEL_STACKS[0]) as *mut ThreadInfo;
        (*ti).task = core::ptr::null_mut();
        (*ti).task_pid = 0;
        (*ti).cpu_id = 0;
        (*ti).flags = 0;
        (*ti).preempt_count = 0;
        (*ti).canary = STACK_CANARY;
    }
}

#[unsafe(no_mangle)]
#[unsafe(link_section = ".init.text")]
pub unsafe fn init_scheduler_for_cpu(cpu_id: usize) {
    pr_info!("Initializing scheduler for CPU {}", cpu_id);
    let boot_cr3 = paging::bootstrap_directory_phys_addr();
    let k_stack_bottom = (unsafe { &raw const IDLE_KERNEL_STACKS[cpu_id] }) as u32;
    let k_stack_top = k_stack_bottom + THREAD_SIZE as u32;
    debug_assert_eq!(k_stack_bottom % THREAD_SIZE as u32, 0); // catch this class of bug immediately if it regresses

    let mut idle_task: TaskStruct = unsafe { core::mem::MaybeUninit::zeroed().assume_init() };
    idle_task.pid = cpu_id as u32;
    let mut comm = *b"swapper/\0\0\0\0\0\0\0\0";
    let mut len = 8;
    if cpu_id >= 10 {
        comm[len] = b'0' + (cpu_id / 10) as u8;
        len += 1;
    }
    comm[len] = b'0' + (cpu_id % 10) as u8;
    idle_task.set_comm(&comm[..=len]);
    idle_task.credentials = Credentials {
        uid: 0,
        gid: 0,
        euid: 0,
        egid: 0,
        fsuid: 0,
        fsgid: 0,
        groups: [0; 8],
        group_count: 0,
    };
    idle_task.state = ProcessState::Running;
    idle_task.context.cr3 = boot_cr3;
    idle_task.kernel_stack_top = k_stack_top;
    idle_task.kernel_stack_bottom = k_stack_bottom;

    let mut table = PROCESS_TABLE.lock();
    table[cpu_id] = Some(idle_task);

    let ti = k_stack_bottom as *mut ThreadInfo;
    let physical_address_of_task_struct = table[cpu_id].as_mut().unwrap() as *mut TaskStruct;
    pr_debug!(
        "Setting ThreadInfo.task for CPU {} to {:#X}",
        cpu_id,
        physical_address_of_task_struct as u32
    );
    unsafe {
        (*ti).task_pid = cpu_id as u32;
        (*ti).task = physical_address_of_task_struct;
        (*ti).cpu_id = cpu_id as u32;
        (*ti).preempt_count = 0;
        (*ti).flags = 0;
        (*ti).canary = thread_info::STACK_CANARY;
    }

    pr_debug!(
        "Scheduler initialized idle task for CPU {} with PID {} at {:#X}",
        cpu_id,
        cpu_id,
        physical_address_of_task_struct as u32
    );

    gdt::set_kernel_stack_for_cpu(cpu_id, k_stack_top);
    pr_debug!("Scheduler initialized idle task for CPU {}", cpu_id);
}

/// Picks the next task for this CPU and returns the kernel ESP to resume
/// (its saved context frame). Delivers a pending signal to the chosen task,
/// and switches CR3 and the TSS's ESP0 to it.
///
/// # Safety
/// Call with interrupts disabled, from an interrupt, syscall or exception
/// path, and without holding PROCESS_TABLE. `old_esp` must be the current
/// task's saved context frame on its kernel stack, or 0 when the current task
/// won't resume (a Zombie). The caller must switch to the returned ESP.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn schedule(old_esp: u32) -> u32 {
    let cpu_id = unsafe { thread_info::current_cpu() } as usize;
    let current_pid = unsafe { thread_info::current_pid() } as usize;
    let current_ticks = timer::get_ticks() as u64;

    let mut table = PROCESS_TABLE.lock();
    let mut due = [0usize; MAX_PROCESSES];
    let due_count = runqueue::take_due_sleepers(current_ticks, &mut due);
    for &pid in &due[..due_count] {
        if let Some(task) = table[pid].as_mut()
            && task.state == ProcessState::Sleeping
        {
            super::set_state(task, ProcessState::Ready);
        }
    }

    if let Some(ref mut current_task) = table[current_pid] {
        match current_task.state {
            ProcessState::Running => {
                current_task.context.esp = old_esp;
                super::set_state(current_task, ProcessState::Ready);
            }
            ProcessState::Sleeping | ProcessState::Waiting => {
                current_task.context.esp = old_esp;
            }
            _ => {}
        }
    }

    loop {
        // Idle tasks are never queued, so another core's idle task can't be
        // picked here; with nothing ready, fall back to this core's own.
        let next_pid = match runqueue::pop_ready() {
            Some(pid)
                if table[pid]
                    .as_ref()
                    .is_some_and(|task| task.state == ProcessState::Ready) =>
            {
                pid
            }
            Some(_) => continue,
            None => cpu_id,
        };

        let kill_info = {
            let next_task = table[next_pid].as_mut().unwrap();
            if let Some(sig_num) = next_task.signals.pop() {
                let handler_addr = next_task.signals.get_handler(sig_num as usize);

                if handler_addr != 0 {
                    let frame = unsafe { &mut *(next_task.context.esp as *mut ContextFrame) };
                    if (frame.cs & 0x03) == 3 {
                        let next_cr3 = next_task.context.cr3;
                        // SAFETY: the task's own page directory, which maps
                        // the kernel too; it's loaded anyway once it runs.
                        // Known gap: `user_esp` isn't validated, so a task
                        // with a bad stack pointer makes this write fault in
                        // the kernel.
                        unsafe {
                            if next_cr3 != x86::read_cr3() {
                                x86::write_cr3(next_cr3);
                            }
                            frame.user_esp -= 4;
                            *(frame.user_esp as *mut u32) = frame.eip;
                        }
                        frame.eip = handler_addr;
                    }
                    None
                } else {
                    pr_info!(
                        "PID {} terminated by unhandled signal {}",
                        next_pid,
                        sig_num
                    );
                    super::set_state(next_task, ProcessState::Zombie);
                    next_task.exit_code = Some(128 + sig_num as u32);
                    let fds_to_close = next_task.fd_tbl;
                    next_task.fd_tbl = [None; MAX_FDS_PER_PROCESS];
                    let vfs_pins = core::mem::take(&mut next_task.vfs_pins);

                    let parent_pid = next_task.family.parent_pid as usize;
                    let child_count = next_task.family.child_count;
                    let mut children = [0u32; MAX_CHILDREN];
                    children[..child_count]
                        .copy_from_slice(&next_task.family.children[..child_count]);
                    next_task.family.child_count = 0;

                    Some((parent_pid, children, child_count, fds_to_close, vfs_pins))
                }
            } else {
                None
            }
        };

        if let Some((parent_pid, children, child_count, fds_to_close, vfs_pins)) = kill_info {
            // Reparent orphans to PID 0 (idle process)
            for &orphan_pid in &children[..child_count] {
                if let Some(ref mut orphan) = table[orphan_pid as usize] {
                    orphan.family.parent_pid = 0;
                }
                if let Some(ref mut init_task) = table[0]
                    && init_task.family.child_count < MAX_CHILDREN
                {
                    init_task.family.children[init_task.family.child_count] = orphan_pid;
                    init_task.family.child_count += 1;
                }
            }

            // Wake the parent if waiting
            if let Some(ref mut parent) = table[parent_pid]
                && parent.state == ProcessState::Waiting
            {
                super::set_state(parent, ProcessState::Ready);
            }

            drop(table);
            unsafe { crate::fs::release_task_pins(vfs_pins) };
            for opt_fd in fds_to_close.iter() {
                if let Some(global_fd) = *opt_fd {
                    unsafe {
                        crate::fs::close_open_file(global_fd);
                    }
                }
            }
            table = PROCESS_TABLE.lock();
            continue; // loop again to pick next ready task
        }

        let next_task = table[next_pid].as_mut().unwrap();
        super::set_state(next_task, ProcessState::Running);
        let next_esp = next_task.context.esp;
        let next_cr3 = next_task.context.cr3;
        let next_kstack_top = next_task.kernel_stack_top;
        let next_kstack_bottom = next_task.kernel_stack_bottom;

        gdt::set_kernel_stack_for_cpu(cpu_id, next_kstack_top);
        if next_cr3 != x86::read_cr3() {
            unsafe { x86::write_cr3(next_cr3) };
        }

        let ti = next_kstack_bottom as *mut ThreadInfo;

        let canary = unsafe { (*ti).canary };
        if canary != STACK_CANARY {
            pr_emerg!(
                "Stack canary mismatch for CPU {}: expected {:#X}, found {:#X}",
                cpu_id,
                STACK_CANARY,
                canary
            );
            panic::save_stack_trace();
            panic::clean_registers_and_halt();
        }

        unsafe {
            (*ti).task_pid = next_pid as u32;
            (*ti).task = next_task as *mut TaskStruct;
            (*ti).cpu_id = cpu_id as u32;
        }

        if next_pid != current_pid {
            super::stats::note_context_switch();
        }
        return next_esp;
    }
}
