use crate::{
    error::{KResult, KernelError},
    fs, paging, pr_info, pr_warn, process_memory,
    sched::{self, ContextFrame, MAX_CHILDREN, PROCESS_TABLE, TaskStruct},
};

pub(super) unsafe fn syscall_fork(regs: *mut ContextFrame) -> u32 {
    let frame = unsafe { &mut *regs };
    match fork(frame) {
        Ok(child_pid) => frame.set_return_value(child_pid),
        Err(error) => frame.set_return_error(error),
    }
    0
}

fn fork(frame: &ContextFrame) -> KResult<u32> {
    // Copy what the checks need, so no reference to the parent is alive while
    // `reserve_process_slot` locks PROCESS_TABLE.
    let (parent_pid, parent_cr3, child_count) = unsafe { sched::current().as_ref() }
        .map(|p| (p.pid, p.context.cr3, p.family.child_count))
        .ok_or(KernelError::Srch)?;
    if child_count >= MAX_CHILDREN {
        pr_warn!(
            "[PID {}] Maximum number of child processes reached",
            parent_pid
        );
        return Err(KernelError::Again);
    }

    let Some(child_pid) = sched::reserve_process_slot() else {
        pr_warn!("[PID {}] No available PID for fork", parent_pid);
        return Err(KernelError::Again);
    };

    let Ok(child_cr3) = (unsafe { paging::clone_address_space(parent_cr3) }) else {
        pr_warn!(
            "[PID {}] Failed to clone address space for child PID {}",
            parent_pid,
            child_pid
        );
        PROCESS_TABLE.lock()[child_pid] = None;
        return Err(KernelError::NoMem);
    };

    let frames_needed = sched::THREAD_SIZE / paging::PAGE_SIZE;
    let child_kstack_phys = match paging::alloc_contiguous_physical_pages_below(
        frames_needed,
        frames_needed,
        paging::PAGE_TABLE_ALLOC_LIMIT,
    ) {
        Ok(frame) => frame,
        Err(e) => {
            pr_warn!(
                "[PID {}] Failed to allocate kernel stack for child PID {}: {:?}",
                parent_pid,
                child_pid,
                e
            );
            unsafe { paging::free_user_address_space(child_cr3) };
            PROCESS_TABLE.lock()[child_pid] = None;
            return Err(KernelError::NoMem);
        }
    };
    let child_kstack_top =
        paging::phys_to_virt(child_kstack_phys) as u32 + sched::THREAD_SIZE as u32;
    let child_kstack_bottom = child_kstack_top - sched::THREAD_SIZE as u32;

    let child_frame_ptr =
        (child_kstack_top - size_of::<ContextFrame>() as u32) as *mut ContextFrame;
    let thread_info = child_kstack_bottom as *mut sched::ThreadInfo;
    unsafe {
        child_frame_ptr.write(*frame);
        (*child_frame_ptr).eax = 0;

        (*thread_info).task_pid = child_pid as u32;
        (*thread_info).cpu_id = sched::current_cpu();
        (*thread_info).preempt_count = 0;
        (*thread_info).flags = 0;
        (*thread_info).canary = sched::STACK_CANARY;
    }

    let cf_eip = frame.eip;
    let cf_cs = frame.cs;
    let cf_eflags = frame.eflags;
    let cf_user_esp = frame.user_esp;
    let cf_user_ss = frame.user_ss;
    pr_info!(
        "Child Frame: EIP={:#x}, CS={:#x}, EFLAGS={:#x}, USER_ESP={:#x}, USER_SS={:#x}",
        cf_eip,
        cf_cs,
        cf_eflags,
        cf_user_esp,
        cf_user_ss
    );

    let mut child_task: TaskStruct = unsafe { core::mem::MaybeUninit::zeroed().assume_init() };
    child_task.pid = child_pid as u32;
    child_task.state = sched::ProcessState::Terminated;
    child_task.exit_code = None;

    // Set the execution pointer to the forged stack
    child_task.context.esp = child_frame_ptr as u32;
    child_task.context.cr3 = child_cr3;
    child_task.kernel_stack_top = child_kstack_top;
    child_task.kernel_stack_bottom = child_kstack_bottom;
    child_task.family.parent_pid = parent_pid;

    {
        let parent = unsafe { sched::current().as_ref() }.ok_or(KernelError::Srch)?;
        child_task.comm = parent.comm;
        child_task.credentials = parent.credentials;
        child_task.memory = parent.memory;
        child_task.cwd = parent.cwd;
        child_task.fd_tbl = parent.fd_tbl;
        process_memory::inherit_environment(parent, &mut child_task);
    }

    // Increment global reference counts for inherited files
    for &global_fd in child_task.fd_tbl.iter().flatten() {
        fs::retain_open_file(global_fd);
        if let Some(open_file) = fs::get_open_file(global_fd) {
            unsafe { fs::vfs_ref_get(open_file.node) };
        }
    }
    let credentials = child_task.credentials;

    let mut table = PROCESS_TABLE.lock();
    if let Some(Some(parent)) = table.get_mut(parent_pid as usize) {
        // Only the parent itself adds children, so the check above still holds.
        parent.family.children[parent.family.child_count] = child_pid as u32;
        parent.family.child_count += 1;
    }
    table[child_pid] = Some(child_task);
    let child = table[child_pid].as_mut().unwrap();
    unsafe { (*thread_info).task = child as *mut TaskStruct };
    sched::set_state(child, sched::ProcessState::Ready);
    drop(table);

    sched::stats::note_process_created(child_pid as u32);
    if let Err(error) = unsafe {
        crate::fs::procfs::process_created(child_pid as u32, credentials.uid, credentials.gid)
    } {
        pr_warn!(
            "Failed to create proc entry for child PID {}: {:?}",
            child_pid,
            error
        );
    }
    Ok(child_pid as u32)
}
