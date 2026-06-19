use crate::gdt::gdt;
use crate::paging::page_table;
use crate::sched;
use sched::task::{ProcessState, TaskStruct};

pub const MAX_PROCESSES: usize = 64;

pub static mut PROCESS_TABLE: [Option<TaskStruct>; MAX_PROCESSES] = {
    const EMPTY: Option<TaskStruct> = None;
    [EMPTY; MAX_PROCESSES]
};

pub static mut CURRENT_PID: usize = 0;

pub fn init_scheduler() {
    unsafe {
        let boot_cr3 = page_table::bootstrap_directory_phys_addr();
        let mut init_task: TaskStruct = core::mem::MaybeUninit::zeroed().assume_init();
        init_task.pid = 0;
        init_task.uid = 0;
        init_task.state = crate::sched::task::ProcessState::Running;
        init_task.context.cr3 = boot_cr3;

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
    crate::x86::write_cr3(next_task.context.cr3);

    let queue = &mut next_task.signals;
    if queue.head != queue.tail {
        let sig_num = queue.pending[queue.tail];
        queue.tail = (queue.tail + 1) % crate::sched::task::SIGNAL_QUEUE_SIZE;

        let handler_addr = queue.handlers[sig_num as usize];
        if handler_addr != 0 {
            // Hijack the stack for custom handlers
            let frame =
                unsafe { &mut *(next_task.context.esp as *mut crate::sched::task::ContextFrame) };
            frame.user_esp -= 4;
            unsafe {
                *(frame.user_esp as *mut u32) = frame.eip;
            }
            frame.eip = handler_addr;
        } else {
            // THE DEFAULT ACTION: Terminate the process cleanly!
            crate::pr_info!(
                "PID {} terminated by unhandled signal {}\n",
                CURRENT_PID,
                sig_num
            );
            next_task.state = crate::sched::task::ProcessState::Zombie;

            // Pivot back to kernel space before abandoning the process
            crate::x86::write_cr3(crate::paging::page_table::bootstrap_directory_phys_addr());

            // Force the scheduler to immediately pick another alive process (like the Shell)
            return schedule(old_esp);
        }
    }
    next_task.context.esp
}
