use crate::sched::{self, ContextFrame};

pub(super) unsafe fn syscall_getuid(regs: *mut ContextFrame) {
    (*regs).set_return_value(sched::current().as_ref().unwrap().credentials.uid);
}
