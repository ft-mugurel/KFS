use crate::sched::{ContextFrame, CURRENT_PID, PROCESS_TABLE};

pub(super) unsafe fn syscall_getuid(regs: *mut ContextFrame) {
    let current_pid = CURRENT_PID as usize;
    let uid = PROCESS_TABLE[current_pid].as_ref().unwrap().uid;
    (*regs).set_return_value(uid);
}
