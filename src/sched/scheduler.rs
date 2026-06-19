use crate::gdt::gdt;
use crate::paging::page_table;
use crate::sched;
use sched::task::{Context, ProcessState, TaskStruct};

pub const MAX_PROCESSES: usize = 64;

pub static mut PROCESS_TABLE: [Option<TaskStruct>; MAX_PROCESSES] = {
    const EMPTY: Option<TaskStruct> = None;
    [EMPTY; MAX_PROCESSES]
};

pub static mut CURRENT_PID: usize = 0;

pub fn init_scheduler() {
    unsafe {
        let boot_cr3 = page_table::bootstrap_directory_phys_addr();
        let init_task = TaskStruct {
            pid: 0,
            parent_pid: 0,
            state: ProcessState::Running,
            context: Context { esp: 0, cr3: boot_cr3 },
            kernel_stack_top: 0,
            kernel_stack_bottom: 0,
            tty_id: 0,
            wakeup_time: 0,
        };

        PROCESS_TABLE[0] = Some(init_task);
        CURRENT_PID = 0;

        crate::pr_info!("Scheduler initialized with PID 0.\n");
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn schedule(old_esp: u32) -> u32 {
    let current_ticks = crate::interrupts::timer::get_ticks();
    // wake up
    for i in 0..MAX_PROCESSES {
        if let Some(ref mut task) = PROCESS_TABLE[i] {
            if task.state == ProcessState::Sleeping && current_ticks >= task.wakeup_time {
                task.state = ProcessState::Ready;
            }
        }
    }

    if let Some(ref mut current_task) = PROCESS_TABLE[CURRENT_PID] {
        if current_task.state == ProcessState::Running {
            current_task.context.esp = old_esp;
            current_task.state = ProcessState::Ready;
        } else if current_task.state == ProcessState::Sleeping {
            current_task.context.esp = old_esp;
        }
    }

    // Round-Robin
    let mut next_pid = CURRENT_PID;
    loop {
        next_pid = (next_pid + 1) % MAX_PROCESSES;

        if let Some(ref task) = PROCESS_TABLE[next_pid] {
            if task.state == ProcessState::Ready {
                break;
            }
        }

        if next_pid == CURRENT_PID {
            next_pid = 0;
            break;
        }
    }

    CURRENT_PID = next_pid;
    let next_task = PROCESS_TABLE[CURRENT_PID].as_mut().unwrap();
    next_task.state = ProcessState::Running;

    gdt::TSS.esp0 = next_task.kernel_stack_top;

    let current_cr3: u32;
    core::arch::asm!("mov {}, cr3", out(reg) current_cr3);

    if next_task.context.cr3 != current_cr3 {
        core::arch::asm!("mov cr3, {}", in(reg) next_task.context.cr3);
    }

    next_task.context.esp
}
