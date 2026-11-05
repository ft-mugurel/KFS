use core::fmt::Write;

use crate::{
    error::{KResult, KernelError},
    fs, paging, sched, security,
};

use super::BufferWriter;

unsafe fn check_process_inspect_permission(task_uid: u32) -> KResult<()> {
    let credentials = sched::current_cred()
        .as_ref()
        .copied()
        .unwrap_or_else(sched::Credentials::root);

    let object = security::SecurityObject::Process { owner_uid: task_uid };
    if security::check(&credentials, &object, security::Operation::Inspect)
        == security::Decision::Deny
    {
        return Err(KernelError::Acces);
    }
    Ok(())
}

pub unsafe fn generate_proc_status(pid: u32, buf: &mut [u8]) -> KResult<usize> {
    let table = sched::PROCESS_TABLE.lock();
    let task = table
        .get(pid as usize)
        .and_then(|opt| opt.as_ref())
        .ok_or(KernelError::Srch)?;

    check_process_inspect_permission(task.credentials.uid)?;

    let mut writer = BufferWriter::new(buf);
    let comm = task.comm();

    let fd_count = task.fd_tbl.iter().flatten().count();

    let _ = writeln!(writer, "Name: {}", comm);
    let _ = writeln!(writer, "State: {}", task.state);
    let _ = writeln!(writer, "Pid: {}", task.pid);
    let _ = writeln!(writer, "PPid: {}", task.family.parent_pid);
    let _ = writeln!(writer, "Uid: {}", task.credentials.uid);
    let _ = writeln!(writer, "Gid: {}", task.credentials.gid);
    let _ = writeln!(writer, "EUid: {}", task.credentials.euid);
    let _ = writeln!(writer, "EGid: {}", task.credentials.egid);
    let _ = writeln!(writer, "FSUid: {}", task.credentials.fsuid);
    let _ = writeln!(writer, "FSGid: {}", task.credentials.fsgid);
    let _ = writeln!(writer, "FDCount: {}", fd_count);
    let _ = writeln!(writer, "ChildCount: {}", task.family.child_count);
    let _ = writeln!(writer, "KernelStackTop: {:#010X}", task.kernel_stack_top);
    let _ = writeln!(
        writer,
        "KernelStackBottom: {:#010X}",
        task.kernel_stack_bottom
    );
    let _ = writeln!(writer, "CR3: {:#010X}", task.context.cr3);
    let _ = writeln!(writer, "ExitCode: {}", task.exit_code.unwrap_or(0));

    Ok(writer.written())
}

pub unsafe fn generate_proc_cmdline(pid: u32, buf: &mut [u8]) -> KResult<usize> {
    let table = sched::PROCESS_TABLE.lock();
    let task = table
        .get(pid as usize)
        .and_then(|opt| opt.as_ref())
        .ok_or(KernelError::Srch)?;

    check_process_inspect_permission(task.credentials.uid)?;

    let mut writer = BufferWriter::new(buf);
    // No argv is kept, so argv[0] is the process name.
    let _ = writer.write_str(task.comm());
    let _ = writer.write_str("\0");
    Ok(writer.written())
}

pub unsafe fn generate_proc_stat(pid: u32, buf: &mut [u8]) -> KResult<usize> {
    let table = sched::PROCESS_TABLE.lock();
    let task = table
        .get(pid as usize)
        .and_then(|opt| opt.as_ref())
        .ok_or(KernelError::Srch)?;

    check_process_inspect_permission(task.credentials.uid)?;

    let mut writer = BufferWriter::new(buf);
    let comm = task.comm();

    let state_char = match task.state {
        sched::ProcessState::Running | sched::ProcessState::Ready => 'R',
        sched::ProcessState::Sleeping | sched::ProcessState::Waiting => 'S',
        sched::ProcessState::Zombie => 'Z',
        sched::ProcessState::Terminated => 'T',
    };

    let _ = writeln!(
        writer,
        "pid: {}\ncomm: {}\nstate: {}\nppid: {}\ncr3: {:#010X}\nesp: {:#010X}",
        task.pid, comm, state_char, task.family.parent_pid, task.context.cr3, task.context.esp
    );

    Ok(writer.written())
}

/// "rwxp"-style permissions for a region. i686 without PAE has no NX bit, so
/// every user page is executable; writability comes from the page tables.
fn perms(writable: bool) -> &'static str {
    if writable { "rwxp" } else { "r-xp" }
}

/// Whether any present page in [base, base + size) of `cr3` is writable
/// regions with nothing mapped yet use `default` (what a fault would map).
fn region_writable(cr3: u32, base: u32, size: u32, default: bool) -> bool {
    let mut page = base & !0xFFF;
    let end = base.saturating_add(size);
    while page < end {
        if let Some(entry) = paging::page_entry_in(cr3, page) {
            return entry & paging::PAGE_WRITABLE != 0;
        }
        page = match page.checked_add(paging::PAGE_SIZE as u32) {
            Some(next) => next,
            None => break,
        };
    }
    default
}

pub unsafe fn generate_proc_maps(pid: u32, buf: &mut [u8]) -> KResult<usize> {
    let table = sched::PROCESS_TABLE.lock();
    let task = table
        .get(pid as usize)
        .and_then(|opt| opt.as_ref())
        .ok_or(KernelError::Srch)?;

    check_process_inspect_permission(task.credentials.uid)?;

    let mut writer = BufferWriter::new(buf);
    let mem = &task.memory;
    let cr3 = task.context.cr3;
    let mut region = |start: u32, end: u32, writable: bool, name: &str| {
        let _ = writeln!(
            writer,
            "{:08x}-{:08x} {} 00000000 00:00 0 {}",
            start,
            end,
            perms(writable),
            name
        );
    };

    // Text and data share the same pages; report them once.
    if mem.code_size > 0 {
        let end = mem.code_base.saturating_add(mem.code_size);
        region(
            mem.code_base,
            end,
            region_writable(cr3, mem.code_base, mem.code_size, true),
            "[text]",
        );
    }
    if mem.data_size > 0 && mem.data_base != mem.code_base {
        let end = mem.data_base.saturating_add(mem.data_size);
        region(
            mem.data_base,
            end,
            region_writable(cr3, mem.data_base, mem.data_size, true),
            "[data]",
        );
    }
    if mem.bss_size > 0 {
        let end = mem.bss_base.saturating_add(mem.bss_size);
        region(
            mem.bss_base,
            end,
            region_writable(cr3, mem.bss_base, mem.bss_size, true),
            "[bss]",
        );
    }
    if mem.heap_brk > mem.heap_base {
        let size = mem.heap_brk - mem.heap_base;
        region(
            mem.heap_base,
            mem.heap_brk,
            region_writable(cr3, mem.heap_base, size, true),
            "[heap]",
        );
    }
    for vma in mem.vmas.iter().filter(|vma| vma.used && vma.size > 0) {
        let name = if vma.base == crate::process_memory::ENV_BASE {
            "[env]"
        } else {
            ""
        };
        region(
            vma.base,
            vma.base.saturating_add(vma.size),
            vma.flags & sched::VMA_WRITE != 0,
            name,
        );
    }
    if mem.stack_base > mem.stack_limit {
        let size = mem.stack_base - mem.stack_limit;
        region(
            mem.stack_limit,
            mem.stack_base,
            region_writable(cr3, mem.stack_limit, size, true),
            "[stack]",
        );
    }

    Ok(writer.written())
}

pub unsafe fn generate_proc_cwd(pid: u32, buf: &mut [u8]) -> KResult<usize> {
    let table = sched::PROCESS_TABLE.lock();
    let task = table
        .get(pid as usize)
        .and_then(|opt| opt.as_ref())
        .ok_or(KernelError::Srch)?;

    check_process_inspect_permission(task.credentials.uid)?;
    fs::node_path(task.cwd, buf)
}
