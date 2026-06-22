use crate::sched::scheduler::{CURRENT_PID, PROCESS_TABLE};
use crate::sched::task::ContextFrame;

pub(super) unsafe fn syscall_getuid(regs: *mut ContextFrame) {
    unsafe {
        let uid = PROCESS_TABLE[CURRENT_PID].as_ref().unwrap().uid;
        (*regs).set_return_value(uid);
    }
}
