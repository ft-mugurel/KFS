use crate::{
    error::KernelError,
    sched::{self, ContextFrame, MAX_PROCESSES, MAX_SIGNALS, PROCESS_TABLE},
};
use crate::security::{self, Decision, Operation};

pub(super) unsafe fn syscall_kill(regs: *mut ContextFrame) {
    unsafe {
        let target_pid = (*regs).arg1() as usize;
        let sig_num = (*regs).arg2() as u8;

        let mut table = PROCESS_TABLE.lock();
        if target_pid >= MAX_PROCESSES || table[target_pid].is_none() {
            (*regs).set_return_error(KernelError::ESRCH);
            return;
        }

        if sig_num >= MAX_SIGNALS as u8 {
            (*regs).set_return_error(KernelError::EINVAL);
            return;
        }

        let caller_credentials = sched::current().as_ref().unwrap().credentials;
        let target_credentials = table[target_pid].as_ref().unwrap().credentials;
        let target_object = security::SecurityObject::Process {
            owner_uid: target_credentials.uid,
        };
        if security::check(&caller_credentials, &target_object, Operation::Signal)
            == Decision::Deny
        {
            (*regs).set_return_error(KernelError::EPERM);
            return;
        }

        let target_task = table[target_pid].as_mut().unwrap();
        if target_task.signals.push(sig_num) {
            (*regs).set_return_value(0);
        } else {
            (*regs).set_return_error(KernelError::EAGAIN);
        }
    }
}

unsafe fn current_signals() -> *mut sched::SignalQueue {
    &mut (*sched::current().as_mut().unwrap()).signals
}

pub(super) unsafe fn syscall_signal(regs: *mut ContextFrame) {
    unsafe {
        let sig_num = (*regs).arg1() as usize;
        let handler_addr = (*regs).arg2(); // The memory address of the user's function

        if sig_num >= MAX_SIGNALS {
            (*regs).set_return_error(KernelError::EINVAL);
            return;
        }

        // Register the user's function pointer in the task struct
        let signals = current_signals();
        (*signals).set_handler(sig_num, handler_addr);

        (*regs).set_return_value(0);
    }
}
