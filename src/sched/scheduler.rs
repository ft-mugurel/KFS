use super::{
    ContextFrame, Credentials, ProcessState, TaskStruct, MAX_PROCESSES, PROCESS_TABLE,
    THREAD_SIZE,
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

pub unsafe fn idle_stack_top(cpu_id: usize) -> u32 {
    let k_stack_bottom = (unsafe { &raw const IDLE_KERNEL_STACKS[cpu_id] }) as u32;
    let k_stack_top = k_stack_bottom + THREAD_SIZE as u32;
    debug_assert_eq!(k_stack_bottom % THREAD_SIZE as u32, 0); // catch this class of bug immediately if it regresses
    k_stack_top
}

#[unsafe(no_mangle)]
pub unsafe fn init_scheduler_for_cpu(cpu_id: usize) {
    pr_info!("Initializing scheduler for CPU {}\n", cpu_id);
    let boot_cr3 = paging::bootstrap_directory_phys_addr();
    let k_stack_bottom = (unsafe { &raw const IDLE_KERNEL_STACKS[cpu_id] }) as u32;
    let k_stack_top = k_stack_bottom + THREAD_SIZE as u32;
    debug_assert_eq!(k_stack_bottom % THREAD_SIZE as u32, 0); // catch this class of bug immediately if it regresses

    let mut idle_task: TaskStruct = core::mem::MaybeUninit::zeroed().assume_init();
    idle_task.pid = cpu_id as u32;
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
    (*ti).task_pid = cpu_id as u32;
    let physical_address_of_task_struct = table[cpu_id].as_mut().unwrap() as *mut TaskStruct;
    pr_debug!(
        "Setting ThreadInfo.task for CPU {} to {:#X}\n",
        cpu_id,
        physical_address_of_task_struct as u32
    );
    (*ti).task = physical_address_of_task_struct;
    (*ti).cpu_id = cpu_id as u32;
    (*ti).preempt_count = 0;
    (*ti).flags = 0;
    (*ti).canary = thread_info::STACK_CANARY;

    pr_debug!(
        "Scheduler initialized idle task for CPU {} with PID {} at {:#X}\n",
        cpu_id,
        (*ti).task_pid as u32,
        (*ti).task as u32
    );

    gdt::set_kernel_stack_for_cpu(cpu_id, k_stack_top);
    pr_debug!("Scheduler initialized idle task for CPU {}\n", cpu_id);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn schedule(old_esp: u32) -> u32 {
    let cpu_id = thread_info::current_cpu() as usize;
    let current_pid = thread_info::current_pid() as usize;
    let current_ticks = timer::get_ticks() as u64;

    for task in PROCESS_TABLE.lock().iter_mut() {
        if let Some(ref mut task) = task {
            if task.state == ProcessState::Sleeping && current_ticks >= task.wakeup_time {
                task.state = ProcessState::Ready;
            }
        }
    }

    if let Some(ref mut current_task) = PROCESS_TABLE.lock()[current_pid] {
        match current_task.state {
            ProcessState::Running => {
                current_task.context.esp = old_esp;
                current_task.state = ProcessState::Ready;
            }
            ProcessState::Sleeping | ProcessState::Waiting => {
                current_task.context.esp = old_esp;
            }
            _ => {}
        }
    }

    let mut table = PROCESS_TABLE.lock();
    loop {
        let mut next_pid = current_pid;
        loop {
            next_pid = (next_pid + 1) % MAX_PROCESSES;

            // Never steal another core's reserved idle task.
            if next_pid < MAX_CPUS && next_pid != cpu_id {
                if next_pid == current_pid {
                    next_pid = cpu_id;
                    break;
                }
                continue;
            }

            if let Some(ref task) = table[next_pid] {
                if task.state == ProcessState::Ready {
                    break;
                }
            }

            if next_pid == current_pid {
                next_pid = cpu_id; // fall back to THIS core's idle task, not PID 0
                break;
            }
        }

        let next_task = table[next_pid].as_mut().unwrap();
        let mut killed = false;

        if let Some(sig_num) = next_task.signals.pop() {
            let handler_addr = next_task.signals.get_handler(sig_num as usize);

            if handler_addr != 0 {
                let frame = &mut *(next_task.context.esp as *mut ContextFrame);
                if (frame.cs & 0x03) == 3 {
                    let next_cr3 = next_task.context.cr3;
                    if next_cr3 != x86::read_cr3() {
                        x86::write_cr3(next_cr3);
                    }
                    frame.user_esp -= 4;
                    *(frame.user_esp as *mut u32) = frame.eip;
                    frame.eip = handler_addr;
                }
            } else {
                pr_info!(
                    "PID {} terminated by unhandled signal {}\n",
                    next_pid,
                    sig_num
                );
                next_task.state = ProcessState::Zombie;
                killed = true;
            }
        }

        if killed {
            continue; // table stays locked, loop again
        }

        next_task.state = ProcessState::Running;
        let next_esp = next_task.context.esp;
        let next_cr3 = next_task.context.cr3;
        let next_kstack_top = next_task.kernel_stack_top;
        let next_kstack_bottom = next_task.kernel_stack_bottom;

        gdt::set_kernel_stack_for_cpu(cpu_id, next_kstack_top);
        if next_cr3 != x86::read_cr3() {
            x86::write_cr3(next_cr3);
        }

        let ti = next_kstack_bottom as *mut ThreadInfo;

        if (*ti).canary != STACK_CANARY {
            pr_emerg!(
                "Stack canary mismatch for CPU {}: expected {:#X}, found {:#X}\n",
                cpu_id,
                STACK_CANARY,
                (*ti).canary as u32
            );
            panic::save_stack_trace();
            panic::clean_registers_and_halt();
        }

        (*ti).task_pid = next_pid as u32;
        (*ti).task = next_task as *mut TaskStruct;
        (*ti).cpu_id = cpu_id as u32;

        return next_esp;
    }
}
