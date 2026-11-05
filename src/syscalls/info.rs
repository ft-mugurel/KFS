use super::args::syscall_define;
use crate::sched;

syscall_define! {
    pub(crate) fn syscall_getpid() {
        Ok(unsafe { sched::current_pid() })
    }
}

syscall_define! {
    pub(crate) fn syscall_getppid() {
        let parent = unsafe { sched::current().as_ref() }.map_or(0, |task| task.family.parent_pid);
        Ok(parent)
    }
}
