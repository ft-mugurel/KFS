use crate::error::{KResultExt, KernelError};
use crate::fs::OPEN_FILE_TABLE;
use crate::sched::{
    self, current_pid, ContextFrame, MAX_CHILDREN, MAX_FDS_PER_PROCESS, PROCESS_TABLE,
};
use crate::{paging, pr_info, pr_warn};

pub(super) unsafe fn syscall_fork(regs: *mut ContextFrame) {
    let mut child_pid_opt = None;
    for (i, task_opt) in PROCESS_TABLE.lock().iter().enumerate() {
        if task_opt.is_none() {
            child_pid_opt = Some(i);
            break;
        }
    }

    let parent_task = sched::current().as_mut().unwrap();
    let cc = parent_task.family.child_count;
    if cc >= MAX_CHILDREN {
        pr_warn!(
            "[PID {}] Maximum number of child processes reached\n",
            parent_task.pid
        );
        (*regs).set_return_error(KernelError::EAGAIN);
        return;
    }

    let child_pid = match child_pid_opt {
        Some(pid) => pid,
        None => {
            pr_warn!("[PID {}] No available PID for fork\n", current_pid());
            (*regs).set_return_error(KernelError::EAGAIN);
            return;
        }
    };

    let child_cr3 = match paging::clone_address_space(parent_task.context.cr3) {
        Ok(cr3) => cr3,
        Err(_) => {
            pr_warn!(
                "[PID {}] Failed to clone address space for child PID {}\n",
                parent_task.pid,
                child_pid
            );
            (*regs).set_return_error(KernelError::ENOMEM);
            return;
        }
    };

    let child_kstack_phys = paging::alloc_physical_page()
        .consume_err("fork: could not allocate memory\n")
        .unwrap();
    let child_kstack_top = paging::phys_to_virt(child_kstack_phys) as u32 + 4096;
    let child_kstack_bottom = child_kstack_top - 4096;

    let child_frame_ptr =
        (child_kstack_top - core::mem::size_of::<ContextFrame>() as u32) as *mut ContextFrame;

    let cf_eip = (*child_frame_ptr).eip;
    let cf_cs = (*child_frame_ptr).cs;
    let cf_eflags = (*child_frame_ptr).eflags;
    let cf_user_esp = (*child_frame_ptr).user_esp;
    let cf_user_ss = (*child_frame_ptr).user_ss;
    pr_info!(
        "Child Frame: EIP={:#x}, CS={:#x}, EFLAGS={:#x}, USER_ESP={:#x}, USER_SS={:#x}\n",
        cf_eip,
        cf_cs,
        cf_eflags,
        cf_user_esp,
        cf_user_ss
    );

    core::ptr::copy_nonoverlapping(regs, child_frame_ptr, 1);

    (*child_frame_ptr).eax = 0;

    let mut child_task: sched::TaskStruct = core::mem::MaybeUninit::zeroed().assume_init();
    child_task.pid = child_pid as u32;
    child_task.uid = parent_task.uid;
    child_task.state = sched::ProcessState::Ready;
    child_task.exit_code = None;

    // Set the execution pointer to the forged stack
    child_task.context.esp = child_frame_ptr as u32;
    child_task.context.cr3 = child_cr3;

    child_task.memory = parent_task.memory;
    child_task.kernel_stack_top = child_kstack_top;
    child_task.kernel_stack_bottom = child_kstack_bottom;

    child_task.fd_tbl = parent_task.fd_tbl;

    // Increment global reference counts for inherited files
    for i in 0..MAX_FDS_PER_PROCESS {
        if let Some(global_fd) = child_task.fd_tbl[i] {
            if let Some(open_file) = &mut OPEN_FILE_TABLE[global_fd] {
                open_file.ref_count += 1;
            }
        }
    }

    child_task.family.parent_pid = parent_task.pid;
    parent_task.family.children[parent_task.family.child_count] = child_pid as u32;
    parent_task.family.child_count += 1;

    PROCESS_TABLE.lock()[child_pid] = Some(child_task);

    (*regs).set_return_value(child_pid as u32);
}
