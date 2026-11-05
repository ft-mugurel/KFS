use super::args::{UserAddr, syscall_define};
use crate::security::{self, Decision, Operation};
use crate::{
    error::KernelError,
    sched::{self, MAX_SIGNALS, PROCESS_TABLE},
};

syscall_define! {
    pub(super) fn syscall_kill(target_pid: usize, sig_num: u32) {
        // Copy the caller's credentials before taking PROCESS_TABLE: the
        // table guard also covers the caller's own entry, so a reference
        // through `current()` must not be alive while it is held.
        let caller = unsafe { sched::current().as_ref() }.ok_or(KernelError::Srch)?.credentials;

        let mut table = PROCESS_TABLE.lock();
        let target = table.get_mut(target_pid).and_then(Option::as_mut).ok_or(KernelError::Srch)?;

        // Taken as u32 and range-checked: an `as u8` cast would turn
        // kill(pid, 265) into signal 9.
        if sig_num >= MAX_SIGNALS as u32 {
            return Err(KernelError::Inval);
        }

        let target_object = security::SecurityObject::Process { owner_uid: target.credentials.uid };
        if security::check(&caller, &target_object, Operation::Signal) == Decision::Deny {
            return Err(KernelError::Perm);
        }

        if sig_num == 0 {
            // POSIX: signal 0 is a null signal used for error checking (existence and permissions).
            return Ok(0);
        }

        if target.signals.push(sig_num as u8) {
            Ok(0)
        } else {
            Err(KernelError::Again)
        }
    }
}

syscall_define! {
    pub(super) fn syscall_signal(sig_num: usize, handler: UserAddr) {
        if sig_num >= MAX_SIGNALS {
            return Err(KernelError::Inval);
        }
        unsafe { sched::current().as_mut() }
            .ok_or(KernelError::Srch)?
            .signals
            .set_handler(sig_num, handler.0);
        Ok(0)
    }
}
