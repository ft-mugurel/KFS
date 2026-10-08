use super::thread_info;
use super::{
    ContextFrame, Credentials, ProcessState, TaskStruct, EMPTY_VMA, MAX_VMAS, PROCESS_TABLE,
};
use crate::error::KResult;
use crate::gdt::{USER_CODE_SEL, USER_DATA_SEL};
use crate::{paging, pr_err, x86};
use crate::process_memory::{self, ENV_BASE, ENV_MAX_SIZE};
use core::mem::{size_of, MaybeUninit};
use core::ptr::{copy_nonoverlapping, write_bytes};

const USER_CODE_VADDR: u32 = 0x08048000;
const USER_STACK_VADDR: u32 = 0xBFFFF000;

pub unsafe fn create_user_process(
    entry_point: unsafe fn(),
    entry_size: usize,
) -> KResult<usize> {
    let pid = super::reserve_process_slot();
    if pid.is_none() {
        pr_err!("No available PID for new user process\n");
        return Err(crate::error::KernelError::ENOMEM);
    }
    let pid = pid.unwrap();
    let frames_needed = super::THREAD_SIZE / paging::PAGE_SIZE;
    let k_stack_frame = match paging::alloc_contiguous_physical_pages_below(
        frames_needed,
        frames_needed,
        paging::PAGE_TABLE_ALLOC_LIMIT,
    ) {
        Ok(frame) => frame,
        Err(e) => {
            pr_err!(
                "Failed to allocate kernel stack frame for user process: {:?}\n",
                e
            );
            PROCESS_TABLE.lock()[pid] = None; // Free the reserved slot
            return Err(e);
        }
    };
    let k_stack_bottom = paging::phys_to_virt(k_stack_frame) as u32;
    let k_stack_top = k_stack_bottom + super::THREAD_SIZE as u32;

    let frame_ptr = (k_stack_top - size_of::<ContextFrame>() as u32) as *mut ContextFrame;
    write_bytes(frame_ptr, 0, 1);

    let old_cr3 = x86::read_cr3();
    let new_cr3 = match paging::clone_address_space(old_cr3) {
        Ok(cr3) => cr3,
        Err(e) => {
            pr_err!("Failed to create address space for user process: {:?}\n", e);
            let _ = paging::free_contiguous_physical_pages(k_stack_frame, frames_needed);
            PROCESS_TABLE.lock()[pid] = None;
            return Err(e);
        }
    };

    x86::disable_interrupts();

    x86::write_cr3(new_cr3);

    let fail_cleanup = |unmapped_frame: Option<u32>, err: crate::error::KernelError| -> crate::error::KernelError {
        x86::write_cr3(old_cr3);
        if let Some(frame) = unmapped_frame {
            let _ = paging::free_physical_page(frame);
        }
        paging::free_user_address_space(new_cr3);
        let _ = paging::free_contiguous_physical_pages(k_stack_frame, frames_needed);
        PROCESS_TABLE.lock()[pid] = None;
        x86::enable_interrupts();
        err
    };

    let code_pages = (entry_size + 12 + paging::PAGE_SIZE - 1) / paging::PAGE_SIZE;
    let code_pages = if code_pages == 0 { 1 } else { code_pages };
    let user_flags = paging::PAGE_PRESENT | paging::PAGE_USER | paging::PAGE_WRITABLE;
    for i in 0..code_pages {
        let code_frame = match paging::alloc_physical_page_below(paging::PAGE_TABLE_ALLOC_LIMIT) {
            Ok(frame) => frame,
            Err(e) => return Err(fail_cleanup(None, e)),
        };
        if let Err(e) = paging::map_page(USER_CODE_VADDR + (i as u32 * 4096), code_frame, user_flags) {
            return Err(fail_cleanup(Some(code_frame), e));
        }
    }

    let stack_pages = 16u32;
    for i in 1..=stack_pages {
        let stack_frame = match paging::alloc_physical_page_below(paging::PAGE_TABLE_ALLOC_LIMIT) {
            Ok(frame) => frame,
            Err(e) => return Err(fail_cleanup(None, e)),
        };
        if let Err(e) = paging::map_page(USER_STACK_VADDR - (i * 4096), stack_frame, user_flags) {
            return Err(fail_cleanup(Some(stack_frame), e));
        }
    }

    let bss_start_vaddr = USER_CODE_VADDR + (code_pages as u32 * 4096);
    let bss_pages = 8u32;
    for i in 0..bss_pages {
        let bss_frame = match paging::alloc_physical_page_below(paging::PAGE_TABLE_ALLOC_LIMIT) {
            Ok(frame) => frame,
            Err(e) => return Err(fail_cleanup(None, e)),
        };
        if let Err(e) = paging::map_page(bss_start_vaddr + (i * 4096), bss_frame, user_flags) {
            return Err(fail_cleanup(Some(bss_frame), e));
        }
        write_bytes((bss_start_vaddr + (i * 4096)) as *mut u8, 0, 4096);
    }

    for i in 0..(ENV_MAX_SIZE / paging::PAGE_SIZE) {
        let env_frame = match paging::alloc_physical_page_below(paging::PAGE_TABLE_ALLOC_LIMIT) {
            Ok(frame) => frame,
            Err(e) => return Err(fail_cleanup(None, e)),
        };
        if let Err(e) = paging::map_page(
            ENV_BASE + (i as u32 * paging::PAGE_SIZE as u32),
            env_frame,
            user_flags,
        ) {
            return Err(fail_cleanup(Some(env_frame), e));
        }
        write_bytes(
            (ENV_BASE + (i as u32 * paging::PAGE_SIZE as u32)) as *mut u8,
            0,
            paging::PAGE_SIZE,
        );
    }

    let code_ptr = USER_CODE_VADDR as *mut u8;
    copy_nonoverlapping(entry_point as *const u8, code_ptr, entry_size);

    // exit even if the user didn't call exit, to avoid returning to the kernel
    let trampoline_vaddr = USER_CODE_VADDR + entry_size as u32;
    let trampoline_ptr = trampoline_vaddr as *mut u8;
    let exit_payload: [u8; 12] = [
        0xB8, 0x01, 0x00, 0x00, 0x00, // mov eax, 1
        0xBB, 0x00, 0x00, 0x00, 0x00, // mov ebx, 0
        0xCD, 0x80, // int 0x80
    ];
    copy_nonoverlapping(exit_payload.as_ptr(), trampoline_ptr, 12);

    let stack_top_ptr = (USER_STACK_VADDR - 4) as *mut u32;
    stack_top_ptr.write(trampoline_vaddr);

    // Ring 3 Execution Frame
    let frame = &mut *frame_ptr;
    let user_data = (USER_DATA_SEL | 3) as u32;

    frame.ds = user_data;
    frame.es = user_data;
    frame.fs = user_data;
    frame.gs = user_data;

    frame.eip = USER_CODE_VADDR;
    frame.cs = (USER_CODE_SEL | 3) as u32;
    frame.eflags = 0x202;

    frame.user_esp = USER_STACK_VADDR - 4;
    frame.user_ss = user_data;

    let parent_opt = super::current().as_ref();
    let credentials = parent_opt
        .map(|p| p.credentials)
        .unwrap_or_else(Credentials::root);

    let mut new_task: TaskStruct = MaybeUninit::zeroed().assume_init();
    new_task.pid = pid as u32;
    new_task.credentials = credentials;
    new_task.state = ProcessState::Terminated;
    new_task.context.esp = frame_ptr as u32;
    new_task.context.cr3 = new_cr3;

    // Memory Tracking Initialization
    new_task.memory.code_base = USER_CODE_VADDR;
    new_task.memory.code_size = (code_pages as u32) * 4096;
    new_task.memory.data_base = USER_CODE_VADDR;
    new_task.memory.data_size = (code_pages as u32) * 4096;
    new_task.memory.bss_base = bss_start_vaddr;
    new_task.memory.bss_size = bss_pages * 4096;
    new_task.memory.stack_base = USER_STACK_VADDR;
    new_task.memory.stack_limit = USER_STACK_VADDR - (stack_pages * 4096);
    new_task.memory.heap_base = 0x4000_0000;
    new_task.memory.heap_brk = 0x4000_0000;
    new_task.memory.vmas = [EMPTY_VMA; MAX_VMAS];
    new_task.memory.vmas[0] = crate::sched::Vma {
        base: ENV_BASE,
        size: ENV_MAX_SIZE as u32,
        flags: user_flags,
        used: true,
    };
    if let Err(e) = process_memory::init_default_environment(&mut new_task) {
        return Err(fail_cleanup(None, e));
    }

    x86::write_cr3(old_cr3);

    new_task.kernel_stack_top = k_stack_top;
    new_task.kernel_stack_bottom = k_stack_bottom;

    if let Some(parent_task) = parent_opt {
        new_task.cwd = parent_task.cwd;
        new_task.fd_tbl = parent_task.fd_tbl;
        for global_fd in new_task.fd_tbl.iter().flatten() {
            crate::fs::retain_open_file(*global_fd);
        }
    } else {
        new_task.cwd = crate::fs::ROOT_NODE;
        new_task.fd_tbl = [None; super::MAX_FDS_PER_PROCESS];
    }

    let mut locked_process_table = PROCESS_TABLE.lock();
    locked_process_table[pid] = Some(new_task);

    // Getting rid of global CURRENT_PID and using the current thread info to get the PID
    let ti = k_stack_bottom as *mut thread_info::ThreadInfo;
    (*ti).task = locked_process_table[pid].as_mut().unwrap() as *mut _;
    (*ti).task_pid = pid as u32;
    (*ti).cpu_id = super::current_cpu();
    (*ti).preempt_count = 0;
    (*ti).flags = 0;
    (*ti).canary = thread_info::STACK_CANARY;

    // Add the new process to the parent's child list
    let parent_pid = parent_opt.map_or(0, |p| p.pid as usize);
    if let Some(Some(parent_task)) = locked_process_table.get_mut(parent_pid) {
        if parent_task.family.child_count < super::MAX_CHILDREN {
            parent_task.family.children[parent_task.family.child_count] = pid as u32;
            parent_task.family.child_count += 1;
        }
    }

    if parent_opt.is_some() {
        locked_process_table[pid].as_mut().unwrap().state = ProcessState::Ready;
    }
    let credentials = locked_process_table[pid].as_ref().unwrap().credentials;
    drop(locked_process_table);

    x86::enable_interrupts();
    if let Err(error) =
        crate::fs::procfs::process_created(pid as u32, credentials.uid, credentials.gid)
    {
        pr_err!("Failed to create proc entry for PID {}: {:?}\n", pid, error);
    }
    Ok(pid)
}

pub unsafe fn bind_process_to_tty(pid: usize, tty_dev: &str) -> KResult<()> {
    let tty_node = crate::fs::resolve_path(tty_dev, crate::fs::ROOT_NODE)?;
    if (*tty_node).node_type != crate::fs::VfsNodeType::CharDevice {
        return Err(crate::error::KernelError::ENOTTY);
    }

    let global_fd = crate::fs::alloc_open_file(tty_node, 3)?;
    let _ = crate::fs::retain_open_file(global_fd);
    let _ = crate::fs::retain_open_file(global_fd);

    let mut table = PROCESS_TABLE.lock();
    let task = match table.get_mut(pid).and_then(|opt| opt.as_mut()) {
        Some(t) => t,
        None => {
            crate::fs::close_open_file(global_fd);
            crate::fs::close_open_file(global_fd);
            crate::fs::close_open_file(global_fd);
            return Err(crate::error::KernelError::ESRCH);
        }
    };

    for fd in 0..3 {
        if let Some(old_gfd) = task.fd_tbl[fd].take() {
            crate::fs::close_open_file(old_gfd);
        }
    }

    task.fd_tbl[0] = Some(global_fd);
    task.fd_tbl[1] = Some(global_fd);
    task.fd_tbl[2] = Some(global_fd);
    task.state = ProcessState::Ready;

    Ok(())
}

pub unsafe fn start_process(pid: usize) -> KResult<()> {
    let mut table = PROCESS_TABLE.lock();
    let task = match table.get_mut(pid).and_then(|opt| opt.as_mut()) {
        Some(t) => t,
        None => return Err(crate::error::KernelError::ESRCH),
    };
    task.state = ProcessState::Ready;
    Ok(())
}

pub unsafe fn create_user_process_on_tty(
    entry_point: unsafe fn(),
    entry_size: usize,
    tty_dev: &str,
) -> KResult<usize> {
    let pid = create_user_process(entry_point, entry_size)?;
    bind_process_to_tty(pid, tty_dev)?;
    Ok(pid)
}

pub static USER_SHELL_PAYLOAD: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/build/sh.bin"));

pub unsafe fn spawn_init_shell() -> KResult<usize> {
    let dummy_fn: unsafe fn() = core::mem::transmute(USER_SHELL_PAYLOAD.as_ptr());
    create_user_process_on_tty(dummy_fn, USER_SHELL_PAYLOAD.len(), "/dev/tty1")
}

pub unsafe fn spawn_init_shells() -> KResult<()> {
    let dummy_fn: unsafe fn() = core::mem::transmute(USER_SHELL_PAYLOAD.as_ptr());
    let ttys = [
        "/dev/tty1",
        "/dev/tty2",
        "/dev/tty3",
        "/dev/tty4",
        "/dev/tty5",
        "/dev/tty6",
    ];
    for tty in ttys {
        match create_user_process_on_tty(dummy_fn, USER_SHELL_PAYLOAD.len(), tty) {
            Ok(pid) => {
                crate::pr_debug!("Spawned user getty/shell on {} (PID {})\n", tty, pid);
            }
            Err(e) => {
                crate::pr_err!("Failed to spawn shell on {}: {:?}\n", tty, e);
            }
        }
    }
    Ok(())
}
