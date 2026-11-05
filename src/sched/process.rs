use super::thread_info;
use super::{
    ContextFrame, Credentials, EMPTY_VMA, MAX_VMAS, PROCESS_TABLE, ProcessState, TaskStruct,
};
use crate::error::KResult;
use crate::gdt::{USER_CODE_SEL, USER_DATA_SEL};
use crate::process_memory::{self, ENV_BASE, ENV_MAX_SIZE};
use crate::{paging, pr_err, x86};
use core::mem::{MaybeUninit, size_of};
use core::ptr::{copy_nonoverlapping, write_bytes};

const USER_CODE_VADDR: u32 = 0x08048000;
const USER_STACK_VADDR: u32 = 0xBFFFF000;

/// What a new process inherits from the task that creates it, copied out so
/// no reference to that task is alive while PROCESS_TABLE is locked.
struct ParentInfo {
    pid: u32,
    credentials: Credentials,
    cwd: *mut crate::fs::VfsNode,
    fd_tbl: [Option<usize>; super::MAX_FDS_PER_PROCESS],
}

/// Creates a ring-3 process running `program` (raw machine code loaded at
/// `USER_CODE_VADDR`), with its own address space, kernel stack, stack, BSS
/// and environment. A process created from a user task inherits its
/// credentials, cwd and descriptors and is made Ready; one created from
/// kernel context stays not-Ready until the caller finishes setting it up.
///
/// # Safety
/// Call from kernel context with no spinlock held: it switches CR3 to the
/// new address space with interrupts disabled while filling it, and briefly
/// takes PROCESS_TABLE. The root filesystem must be mounted (`ROOT_NODE` is
/// the cwd of processes created from kernel context).
pub unsafe fn create_user_process(name: &str, program: &[u8]) -> KResult<usize> {
    let pid = super::reserve_process_slot();
    if pid.is_none() {
        pr_err!("No available PID for new user process");
        return Err(crate::error::KernelError::NoMem);
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
                "Failed to allocate kernel stack frame for user process: {:?}",
                e
            );
            PROCESS_TABLE.lock()[pid] = None; // Free the reserved slot
            return Err(e);
        }
    };
    let k_stack_bottom = paging::phys_to_virt(k_stack_frame) as u32;
    let k_stack_top = k_stack_bottom + super::THREAD_SIZE as u32;

    let frame_ptr = (k_stack_top - size_of::<ContextFrame>() as u32) as *mut ContextFrame;
    unsafe { write_bytes(frame_ptr, 0, 1) };

    let old_cr3 = x86::read_cr3();
    let new_cr3 = match unsafe { paging::clone_address_space(old_cr3) } {
        Ok(cr3) => cr3,
        Err(e) => {
            pr_err!("Failed to create address space for user process: {:?}", e);
            let _ = paging::free_contiguous_physical_pages(k_stack_frame, frames_needed);
            PROCESS_TABLE.lock()[pid] = None;
            return Err(e);
        }
    };

    // Fill the new address space while it is loaded. Interrupts stay off until
    // CR3 is back to `old_cr3`, so nothing else runs in it meanwhile.
    x86::disable_interrupts();
    unsafe { x86::write_cr3(new_cr3) };

    let fail_cleanup = |unmapped_frame: Option<u32>,
                        err: crate::error::KernelError|
     -> crate::error::KernelError {
        // SAFETY: back to the directory this CPU started on; the new one is
        // freed only after that, and nothing else ever loaded it. Interrupts
        // were enabled on entry (this function's contract).
        unsafe {
            x86::write_cr3(old_cr3);
            if let Some(frame) = unmapped_frame {
                let _ = paging::free_physical_page(frame);
            }
            paging::free_user_address_space(new_cr3);
            let _ = paging::free_contiguous_physical_pages(k_stack_frame, frames_needed);
            PROCESS_TABLE.lock()[pid] = None;
            x86::enable_interrupts();
        }
        err
    };

    // + 12 for the exit stub appended after the program.
    let code_pages = ((program.len() + 12).div_ceil(paging::PAGE_SIZE)).max(1);
    let user_flags = paging::PAGE_PRESENT | paging::PAGE_USER | paging::PAGE_WRITABLE;
    for i in 0..code_pages {
        let code_frame = match paging::alloc_physical_page_below(paging::PAGE_TABLE_ALLOC_LIMIT) {
            Ok(frame) => frame,
            Err(e) => return Err(fail_cleanup(None, e)),
        };
        if let Err(e) =
            paging::map_page(USER_CODE_VADDR + (i as u32 * 4096), code_frame, user_flags)
        {
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
        let vaddr = bss_start_vaddr + (i * 4096);
        if let Err(e) = paging::map_page(vaddr, bss_frame, user_flags) {
            return Err(fail_cleanup(Some(bss_frame), e));
        }
        unsafe { write_bytes(vaddr as *mut u8, 0, 4096) };
    }

    for i in 0..(ENV_MAX_SIZE / paging::PAGE_SIZE) {
        let env_frame = match paging::alloc_physical_page_below(paging::PAGE_TABLE_ALLOC_LIMIT) {
            Ok(frame) => frame,
            Err(e) => return Err(fail_cleanup(None, e)),
        };
        let vaddr = ENV_BASE + (i as u32 * paging::PAGE_SIZE as u32);
        if let Err(e) = paging::map_page(vaddr, env_frame, user_flags) {
            return Err(fail_cleanup(Some(env_frame), e));
        }
        unsafe { write_bytes(vaddr as *mut u8, 0, paging::PAGE_SIZE) };
    }

    // exit even if the user didn't call exit, to avoid returning to the kernel
    let trampoline_vaddr = USER_CODE_VADDR + program.len() as u32;
    let exit_payload: [u8; 12] = [
        0xB8, 0x01, 0x00, 0x00, 0x00, // mov eax, 1
        0xBB, 0x00, 0x00, 0x00, 0x00, // mov ebx, 0
        0xCD, 0x80, // int 0x80
    ];
    unsafe {
        copy_nonoverlapping(program.as_ptr(), USER_CODE_VADDR as *mut u8, program.len());
        copy_nonoverlapping(
            exit_payload.as_ptr(),
            trampoline_vaddr as *mut u8,
            exit_payload.len(),
        );
        ((USER_STACK_VADDR - 4) as *mut u32).write(trampoline_vaddr);
    }

    // Ring 3 Execution Frame
    let frame = unsafe { &mut *frame_ptr };
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

    // idle tasks (kmain runs as CPU 0's) are not real parents: children spawned
    // from kernel context start unparented and stay not-Ready until the caller
    // (e.g. bind_process_to_tty) finishes setting them up
    let parent = unsafe { super::current().as_ref() }
        .filter(|p| p.pid as usize >= crate::smp::MAX_CPUS)
        .map(|p| ParentInfo {
            pid: p.pid,
            credentials: p.credentials,
            cwd: p.cwd,
            fd_tbl: p.fd_tbl,
        });
    let credentials = parent
        .as_ref()
        .map(|p| p.credentials)
        .unwrap_or_else(Credentials::root);

    // SAFETY: every TaskStruct field is valid as all zeroes: integers, raw
    // pointers (null), `Option<usize>`/`Option<u32>` (None), `bool` (false)
    // and `ProcessState` (0 is `Ready`, overwritten just below).
    let mut new_task: TaskStruct = unsafe { MaybeUninit::zeroed().assume_init() };
    new_task.pid = pid as u32;
    new_task.set_comm(name.as_bytes());
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
        flags: super::VMA_READ | super::VMA_WRITE,
        used: true,
    };
    if let Err(e) = unsafe { process_memory::init_default_environment(&mut new_task) } {
        return Err(fail_cleanup(None, e));
    }

    unsafe { x86::write_cr3(old_cr3) };

    new_task.kernel_stack_top = k_stack_top;
    new_task.kernel_stack_bottom = k_stack_bottom;

    if let Some(parent) = &parent {
        new_task.cwd = parent.cwd;
        new_task.fd_tbl = parent.fd_tbl;
        for global_fd in new_task.fd_tbl.iter().flatten() {
            crate::fs::retain_open_file(*global_fd);
        }
    } else {
        new_task.cwd = unsafe { crate::fs::ROOT_NODE };
        new_task.fd_tbl = [None; super::MAX_FDS_PER_PROCESS];
    }

    let mut locked_process_table = PROCESS_TABLE.lock();
    locked_process_table[pid] = Some(new_task);

    // Getting rid of global CURRENT_PID and using the current thread info to get the PID
    let ti = k_stack_bottom as *mut thread_info::ThreadInfo;
    let task_ptr = locked_process_table[pid].as_mut().unwrap() as *mut TaskStruct;
    unsafe {
        (*ti).task = task_ptr;
        (*ti).task_pid = pid as u32;
        (*ti).cpu_id = super::current_cpu();
        (*ti).preempt_count = 0;
        (*ti).flags = 0;
        (*ti).canary = thread_info::STACK_CANARY;
    }

    // Add the new process to the parent's child list
    let parent_pid = parent.as_ref().map_or(0, |p| p.pid as usize);
    if let Some(Some(parent_task)) = locked_process_table.get_mut(parent_pid)
        && parent_task.family.child_count < super::MAX_CHILDREN
    {
        parent_task.family.children[parent_task.family.child_count] = pid as u32;
        parent_task.family.child_count += 1;
    }

    if parent.is_some() {
        super::set_state(
            locked_process_table[pid].as_mut().unwrap(),
            ProcessState::Ready,
        );
    }
    drop(locked_process_table);

    unsafe { x86::enable_interrupts() };
    super::stats::note_process_created(pid as u32);
    if let Err(error) =
        unsafe { crate::fs::procfs::process_created(pid as u32, credentials.uid, credentials.gid) }
    {
        pr_err!("Failed to create proc entry for PID {}: {:?}", pid, error);
    }
    Ok(pid)
}

pub unsafe fn bind_process_to_tty(pid: usize, tty_dev: &str) -> KResult<()> {
    let global_fd = {
        let _pin = crate::fs::pin_nodes();
        let tty_node = unsafe { crate::fs::resolve_path(tty_dev, crate::fs::ROOT_NODE) }?;
        if unsafe { (*tty_node).node_type } != crate::fs::VfsNodeType::CharDevice {
            return Err(crate::error::KernelError::NotTTY);
        }
        crate::fs::alloc_open_file(tty_node, 3)?
    };

    let mut table = PROCESS_TABLE.lock();
    let Some(task) = table.get_mut(pid).and_then(|opt| opt.as_mut()) else {
        drop(table);
        unsafe {
            crate::fs::close_open_file(global_fd);
            crate::fs::close_open_file(global_fd);
            crate::fs::close_open_file(global_fd);
        }
        return Err(crate::error::KernelError::Srch);
    };

    let mut old_files = [None; 3];
    for (old, slot) in old_files.iter_mut().zip(task.fd_tbl.iter_mut()) {
        *old = slot.replace(global_fd);
    }
    super::set_state(task, ProcessState::Ready);
    drop(table);

    // After unlocking: closing a pipe end takes PIPES.
    for old_gfd in old_files.into_iter().flatten() {
        unsafe { crate::fs::close_open_file(old_gfd) };
    }

    Ok(())
}

pub unsafe fn create_user_process_on_tty(
    name: &str,
    program: &[u8],
    tty_dev: &str,
) -> KResult<usize> {
    let pid = unsafe { create_user_process(name, program) }?;
    unsafe { bind_process_to_tty(pid, tty_dev) }?;
    Ok(pid)
}

pub static USER_SHELL_PAYLOAD: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/build/sh.bin"));

pub unsafe fn spawn_init_shells() -> KResult<()> {
    let ttys = [
        "/dev/tty1",
        "/dev/tty2",
        "/dev/tty3",
        "/dev/tty4",
        "/dev/tty5",
        "/dev/tty6",
    ];
    for tty in ttys {
        match unsafe { create_user_process_on_tty("sh", USER_SHELL_PAYLOAD, tty) } {
            Ok(pid) => {
                crate::pr_debug!("Spawned user getty/shell on {} (PID {})", tty, pid);
            }
            Err(e) => {
                crate::pr_err!("Failed to spawn shell on {}: {:?}", tty, e);
            }
        }
    }
    Ok(())
}
