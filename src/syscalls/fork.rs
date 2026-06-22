use crate::sched::scheduler::{CURRENT_PID, PROCESS_TABLE};
use crate::sched::task::ContextFrame;

pub(super) unsafe fn syscall_fork(regs: *mut ContextFrame) {
    let parent_pid = CURRENT_PID;

    let mut child_pid_opt = None;
    for i in 1..crate::sched::scheduler::MAX_PROCESSES {
        if PROCESS_TABLE[i].is_none() {
            child_pid_opt = Some(i);
            break;
        }
    }

    let child_pid = match child_pid_opt {
        Some(pid) => pid,
        None => {
            (*regs).set_return_value(!0u32); // EAGAIN
            return;
        }
    };

    let parent_task = PROCESS_TABLE[parent_pid].as_ref().unwrap();

    let child_cr3 = match crate::paging::page_table::clone_address_space(parent_task.context.cr3) {
        Some(cr3) => cr3,
        None => {
            (*regs).set_return_value(!0u32); // ENOMEM
            return;
        }
    };

    let child_kstack_phys = crate::paging::physical::alloc_physical_page().unwrap();
    let child_kstack_top = crate::paging::page_table::phys_to_virt(child_kstack_phys) as u32 + 4096;
    let child_kstack_bottom = child_kstack_top - 4096;

    let child_frame_ptr =
        (child_kstack_top - core::mem::size_of::<ContextFrame>() as u32) as *mut ContextFrame;

    let cf_eip = (*child_frame_ptr).eip;
    let cf_cs = (*child_frame_ptr).cs;
    let cf_eflags = (*child_frame_ptr).eflags;
    let cf_user_esp = (*child_frame_ptr).user_esp;
    let cf_user_ss = (*child_frame_ptr).user_ss;
    crate::pr_info!(
        "Child Frame: EIP={:#x}, CS={:#x}, EFLAGS={:#x}, USER_ESP={:#x}, USER_SS={:#x}\n",
        cf_eip,
        cf_cs,
        cf_eflags,
        cf_user_esp,
        cf_user_ss
    );

    core::ptr::copy_nonoverlapping(regs, child_frame_ptr, 1);

    (*child_frame_ptr).eax = 0;

    let mut child_task: crate::sched::task::TaskStruct =
        core::mem::MaybeUninit::zeroed().assume_init();
    child_task.pid = child_pid as u32;
    child_task.uid = parent_task.uid;
    child_task.state = crate::sched::task::ProcessState::Ready;

    // Set the execution pointer to the forged stack
    child_task.context.esp = child_frame_ptr as u32;
    child_task.context.cr3 = child_cr3;

    child_task.memory = parent_task.memory;
    child_task.kernel_stack_top = child_kstack_top;
    child_task.kernel_stack_bottom = child_kstack_bottom;

    for i in 0..crate::sched::task::MAX_FDS_PER_PROCESS {
        if let Some(fd) = parent_task.fd_tbl[i] {
            child_task.fd_tbl[i] = Some(fd);
            if let crate::fs::vfs::FileDescriptor::Socket(sock_idx) = fd {
                crate::ipc::SOCKETS[sock_idx].lock().ref_count += 1;
            }
        }
    }

    child_task.family.parent_pid = parent_pid as u32;

    let parent_task_mut = PROCESS_TABLE[parent_pid].as_mut().unwrap();
    let cc = parent_task_mut.family.child_count;
    if cc < crate::sched::task::MAX_CHILDREN {
        parent_task_mut.family.children[cc] = child_pid as u32;
        parent_task_mut.family.child_count += 1;
    }

    PROCESS_TABLE[child_pid] = Some(child_task);

    (*regs).set_return_value(child_pid as u32);
}
