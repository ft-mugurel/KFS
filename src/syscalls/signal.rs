use crate::{
    error::KernelError,
    sched::{
        ContextFrame, CURRENT_PID, MAX_PROCESSES, MAX_SIGNALS, PROCESS_TABLE, SIGNAL_QUEUE_SIZE,
    },
};

pub(super) unsafe fn syscall_kill(regs: *mut ContextFrame) {
    unsafe {
        let target_pid = (*regs).arg1() as usize;
        let sig_num = (*regs).arg2() as u8;

        if target_pid >= MAX_PROCESSES || PROCESS_TABLE[target_pid].is_none() {
            (*regs).set_return_error(KernelError::ESRCH);
            return;
        }

        if sig_num >= MAX_SIGNALS as u8 {
            (*regs).set_return_error(KernelError::EINVAL);
            return;
        }

        let target_task = PROCESS_TABLE[target_pid].as_mut().unwrap();
        let queue = &mut target_task.signals;

        let next_head = (queue.head + 1) % SIGNAL_QUEUE_SIZE;
        if next_head != queue.tail {
            queue.pending[queue.head] = sig_num;
            queue.head = next_head;
            (*regs).set_return_value(0);
        } else {
            (*regs).set_return_error(KernelError::EAGAIN);
        }
    }
}

pub(super) unsafe fn syscall_signal(regs: *mut ContextFrame) {
    unsafe {
        let sig_num = (*regs).arg1() as usize;
        let handler_addr = (*regs).arg2(); // The memory address of the user's function
        let current_pid = CURRENT_PID;

        if sig_num >= MAX_SIGNALS {
            (*regs).set_return_error(KernelError::EINVAL);
            return;
        }

        // Register the user's function pointer in the task struct
        let task = PROCESS_TABLE[current_pid].as_mut().unwrap();
        task.signals.handlers[sig_num] = handler_addr;

        (*regs).set_return_value(0);
    }
}
