use crate::sched::{ContextFrame, CURRENT_PID, PROCESS_TABLE};

pub(super) unsafe fn syscall_getuid(regs: *mut ContextFrame) {
    unsafe {
        let uid = PROCESS_TABLE[CURRENT_PID].as_ref().unwrap().uid;
        (*regs).set_return_value(uid);
    }
}
