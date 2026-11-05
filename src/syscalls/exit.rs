use super::args::syscall_define;
use crate::{
    error::{KResultExt, KernelError},
    fs, paging,
    sched::{self, ContextFrame, MAX_CHILDREN, PROCESS_TABLE, ProcessState},
};

pub unsafe fn syscall_exit(regs: *mut ContextFrame) -> u32 {
    let exit_code = unsafe { (*regs).arg1() };
    let esp = unsafe { terminate_current(exit_code) };

    // DO NOT touch CR3 or free memory here.
    // Memory is preserved until the parent calls waitpid().
    unsafe { sched::schedule(esp) }
}

/// Closes the current task's files, hands its children to PID 0, makes it a
/// Zombie and wakes its parent. Returns its saved ESP; the caller must then
/// schedule away.
pub(crate) unsafe fn terminate_current(exit_code: u32) -> u32 {
    let (pid, family, esp, vfs_pins) = {
        let task = unsafe { sched::current().as_mut() }.unwrap();
        task.exit_code = Some(exit_code);

        // VFS Cleanup
        for slot in task.fd_tbl.iter_mut() {
            if let Some(global_fd) = slot.take() {
                unsafe { fs::close_open_file(global_fd) };
            }
        }
        let vfs_pins = core::mem::take(&mut task.vfs_pins);
        (task.pid as usize, task.family, task.context.esp, vfs_pins)
    };
    unsafe { fs::release_task_pins(vfs_pins) };

    let mut table = PROCESS_TABLE.lock();

    // Reparent orphans to PID 0 (root/idle process)
    for &orphan_pid in &family.children[..family.child_count] {
        if let Some(orphan) = table[orphan_pid as usize].as_mut() {
            orphan.family.parent_pid = 0;
        }
        if let Some(init_task) = table[0].as_mut()
            && init_task.family.child_count < MAX_CHILDREN
        {
            init_task.family.children[init_task.family.child_count] = orphan_pid;
            init_task.family.child_count += 1;
        }
    }

    // Become a Zombie only now, with the table locked: the parent may reap
    // this task as soon as it sees that state.
    if let Some(task) = table[pid].as_mut() {
        task.family.child_count = 0;
        sched::set_state(task, ProcessState::Zombie);
    }

    // Wake the parent if it is blocked in waitpid()
    if let Some(parent) = table[family.parent_pid as usize].as_mut()
        && parent.state == ProcessState::Waiting
    {
        sched::set_state(parent, ProcessState::Ready);
    }
    drop(table);

    esp
}

syscall_define! {
    pub(super) fn syscall_wait() {
        let pid = unsafe { sched::current().as_ref() }.ok_or(KernelError::Srch)?.pid as usize;

        let mut table = PROCESS_TABLE.lock();
        let family = table[pid].as_ref().ok_or(KernelError::Srch)?.family;
        if family.child_count == 0 {
            return Err(KernelError::Child);
        }

        let zombie = (0..family.child_count).find(|&i| {
            table[family.children[i] as usize]
                .as_ref()
                .is_some_and(|child| child.state == ProcessState::Zombie)
        });
        let Some(i) = zombie else {
            sched::set_state(table[pid].as_mut().unwrap(), ProcessState::Waiting);
            return Err(KernelError::Restart);
        };

        let child = table[family.children[i] as usize].take().unwrap();
        let parent = &mut table[pid].as_mut().unwrap().family;
        parent.children[i] = parent.children[parent.child_count - 1];
        parent.child_count -= 1;
        unsafe { crate::fs::procfs::process_reaped(child.pid) };
        drop(table);

        unsafe { paging::free_user_address_space(child.context.cr3) };
        let Some(k_stack_phys) = (unsafe { paging::virt_to_phys(child.kernel_stack_bottom) }) else {
            crate::pr_err!("Failed to translate reaped child kernel stack");
            return Ok(child.pid);
        };
        let frames_needed = sched::THREAD_SIZE / paging::PAGE_SIZE;
        paging::free_contiguous_physical_pages(k_stack_phys, frames_needed)
            .consume_err("Failed to free kernel stack for reaped child process");
        Ok(child.pid)
    }
}
