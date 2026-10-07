use core::fmt::Write;

use crate::{
    error::{KResult, KernelError},
    fs::{self, VfsNode},
    sched, security,
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
        return Err(KernelError::EACCES);
    }
    Ok(())
}

pub unsafe fn generate_proc_status(pid: u32, buf: &mut [u8]) -> KResult<usize> {
    let table = sched::PROCESS_TABLE.lock();
    let task = table
        .get(pid as usize)
        .and_then(|opt| opt.as_ref())
        .ok_or(KernelError::ESRCH)?;

    check_process_inspect_permission(task.credentials.uid)?;

    let mut writer = BufferWriter::new(buf);
    let comm = if pid == 0 {
        "idle"
    } else if pid == 1 {
        "mysh"
    } else {
        "user_proc"
    };

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
    let _ = writeln!(writer, "KernelStackBottom: {:#010X}", task.kernel_stack_bottom);
    let _ = writeln!(writer, "CR3: {:#010X}", task.context.cr3);
    let _ = writeln!(writer, "ExitCode: {}", task.exit_code.unwrap_or(0));

    Ok(writer.written())
}

pub unsafe fn generate_proc_cmdline(pid: u32, buf: &mut [u8]) -> KResult<usize> {
    let table = sched::PROCESS_TABLE.lock();
    let task = table
        .get(pid as usize)
        .and_then(|opt| opt.as_ref())
        .ok_or(KernelError::ESRCH)?;

    check_process_inspect_permission(task.credentials.uid)?;

    let mut writer = BufferWriter::new(buf);
    let comm = if pid == 0 {
        "idle\0"
    } else if pid == 1 {
        "mysh\0"
    } else {
        "user_proc\0"
    };
    let _ = writer.write_str(comm);
    Ok(writer.written())
}

pub unsafe fn generate_proc_stat(pid: u32, buf: &mut [u8]) -> KResult<usize> {
    let table = sched::PROCESS_TABLE.lock();
    let task = table
        .get(pid as usize)
        .and_then(|opt| opt.as_ref())
        .ok_or(KernelError::ESRCH)?;

    check_process_inspect_permission(task.credentials.uid)?;

    let mut writer = BufferWriter::new(buf);
    let comm = if pid == 0 {
        "idle"
    } else if pid == 1 {
        "mysh"
    } else {
        "user_proc"
    };

    let state_char = match task.state {
        sched::ProcessState::Running => 'R',
        sched::ProcessState::Ready => 'D',
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

pub unsafe fn generate_proc_maps(pid: u32, buf: &mut [u8]) -> KResult<usize> {
    let table = sched::PROCESS_TABLE.lock();
    let task = table
        .get(pid as usize)
        .and_then(|opt| opt.as_ref())
        .ok_or(KernelError::ESRCH)?;

    check_process_inspect_permission(task.credentials.uid)?;

    let mut writer = BufferWriter::new(buf);
    let mem = &task.memory;

    if mem.code_size > 0 {
        let _ = writeln!(
            writer,
            "{:08x}-{:08x} r-xp {:08x} [text]",
            mem.code_base,
            mem.code_base.saturating_add(mem.code_size),
            mem.code_size
        );
    }
    if mem.data_size > 0 {
        let _ = writeln!(
            writer,
            "{:08x}-{:08x} rw-p {:08x} [data]",
            mem.data_base,
            mem.data_base.saturating_add(mem.data_size),
            mem.data_size
        );
    }
    if mem.bss_size > 0 {
        let _ = writeln!(
            writer,
            "{:08x}-{:08x} rw-p {:08x} [bss]",
            mem.bss_base,
            mem.bss_base.saturating_add(mem.bss_size),
            mem.bss_size
        );
    }
    if mem.heap_brk > mem.heap_base {
        let _ = writeln!(
            writer,
            "{:08x}-{:08x} rw-p {:08x} [heap]",
            mem.heap_base,
            mem.heap_brk,
            mem.heap_brk - mem.heap_base
        );
    }
    if mem.stack_base > 0 {
        let _ = writeln!(
            writer,
            "{:08x}-{:08x} rwxp {:08x} [stack]",
            mem.stack_base,
            mem.stack_limit,
            mem.stack_limit.saturating_sub(mem.stack_base)
        );
    }

    for vma in &mem.vmas {
        if vma.used && vma.size > 0 {
            let _ = writeln!(
                writer,
                "{:08x}-{:08x} rw-p {:08x} [vma]",
                vma.base,
                vma.base.saturating_add(vma.size),
                vma.size
            );
        }
    }

    Ok(writer.written())
}

pub unsafe fn generate_proc_cwd(pid: u32, buf: &mut [u8]) -> KResult<usize> {
    let table = sched::PROCESS_TABLE.lock();
    let task = table
        .get(pid as usize)
        .and_then(|opt| opt.as_ref())
        .ok_or(KernelError::ESRCH)?;

    check_process_inspect_permission(task.credentials.uid)?;

    let mut current = task.cwd;
    let mut writer = BufferWriter::new(buf);

    if current.is_null() || current == fs::ROOT_NODE {
        let _ = writeln!(writer, "/");
        return Ok(writer.written());
    }

    let mut segments: [*mut VfsNode; 16] = [core::ptr::null_mut(); 16];
    let mut count = 0;

    while !current.is_null() && current != fs::ROOT_NODE && count < 16 {
        segments[count] = current;
        count += 1;
        current = (*current).father;
    }

    for i in (0..count).rev() {
        let _ = write!(writer, "/");
        let node = segments[i];
        let name_len = (*node).name.iter().position(|&c| c == 0).unwrap_or(256);
        let name = core::str::from_utf8(&(&(*node).name)[..name_len]).unwrap_or("?");
        let _ = write!(writer, "{}", name);
    }
    let _ = writeln!(writer);

    Ok(writer.written())
}
