use crate::sched::{self, ContextFrame};

pub(crate) unsafe fn syscall_getpid(regs: *mut ContextFrame) {
    let pid = sched::current_pid();
    (*regs).set_return_value(pid);
}

pub(crate) unsafe fn syscall_getppid(regs: *mut ContextFrame) {
    let task = sched::current().as_ref();
    let ppid = match task {
        Some(t) => t.family.parent_pid,
        None => 0,
    };
    (*regs).set_return_value(ppid);
}
