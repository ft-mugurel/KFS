use super::{
    ContextFrame, ProcessState, TaskStruct, CURRENT_PID, MAX_PROCESSES, PROCESS_TABLE,
    SIGNAL_QUEUE_SIZE,
};
use crate::gdt;
use crate::interrupts::timer;
use crate::paging;
use crate::pr_info;
use crate::x86;
use core::mem::MaybeUninit;

pub fn init_scheduler() {
    unsafe {
        let boot_cr3 = paging::bootstrap_directory_phys_addr();
        let mut init_task: TaskStruct = MaybeUninit::zeroed().assume_init();
        init_task.pid = 0;
        init_task.uid = 0;
        init_task.state = ProcessState::Running;
        init_task.context.cr3 = boot_cr3;

        PROCESS_TABLE[0] = Some(init_task);
        CURRENT_PID = 0;

        pr_info!("Scheduler initialized with PID 0.\n");
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn schedule(old_esp: u32) -> u32 {
    let current_ticks = timer::get_ticks();

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

    loop {
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

        let queue = &mut next_task.signals;
        let mut killed = false;

        if queue.head != queue.tail {
            let sig_num = queue.pending[queue.tail];
            let handler_addr = queue.handlers[sig_num as usize];

            if handler_addr != 0 {
                let frame = &mut *(next_task.context.esp as *mut ContextFrame);
                if (frame.cs & 0x03) == 3 {
                    queue.tail = (queue.tail + 1) % SIGNAL_QUEUE_SIZE;
                    frame.user_esp -= 4;
                    *(frame.user_esp as *mut u32) = frame.eip;
                    frame.eip = handler_addr;
                } else {
                    // Process was in Ring 0
                }
            } else {
                queue.tail = (queue.tail + 1) % SIGNAL_QUEUE_SIZE;
                pr_info!(
                    "PID {} terminated by unhandled signal {}\n",
                    CURRENT_PID as usize,
                    sig_num
                );
                next_task.state = ProcessState::Zombie;
                killed = true;
            }
        }

        if killed {
            continue;
        }

        // Context Switch
        next_task.state = ProcessState::Running;
        gdt::set_kernel_stack(next_task.kernel_stack_top);

        // Streamlined CR3 write
        let current_cr3 = x86::read_cr3();
        if next_task.context.cr3 != current_cr3 {
            x86::write_cr3(next_task.context.cr3);
        }

        return next_task.context.esp;
    }
}

#[unsafe(no_mangle)]
pub unsafe fn yield_cpu() -> ! {
    loop {
        CURRENT_PID = (CURRENT_PID + 1) % MAX_PROCESSES;
        if let Some(ref task) = PROCESS_TABLE[CURRENT_PID] {
            if task.state == ProcessState::Ready {
                break;
            }
        }
    }

    let next_task = PROCESS_TABLE[CURRENT_PID].as_mut().unwrap();
    next_task.state = ProcessState::Running;

    gdt::TSS.esp0 = next_task.kernel_stack_top;
    x86::write_cr3(next_task.context.cr3);

    let next_esp = next_task.context.esp;

    core::arch::asm!(
        "mov esp, {}",
        "popad",
        "pop gs",
        "pop fs",
        "pop es",
        "pop ds",
        "iretd",
        in(reg) next_esp,
        options(noreturn)
    );
}
