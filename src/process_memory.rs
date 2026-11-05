use crate::{
    error::{KResult, KernelError},
    paging,
    sched::TaskStruct,
    x86,
};

pub(crate) const ENV_BASE: u32 = 0x7000_0000;
pub(crate) const ENV_MAX_SIZE: usize = 8 * crate::paging::PAGE_SIZE;
#[expect(dead_code, reason = "argv/envp copy-in for execve")]
pub(crate) const ENV_MAX_ENTRIES: usize = 64;

pub(crate) fn inherit_environment(parent: &TaskStruct, child: &mut TaskStruct) {
    child.env_start = parent.env_start;
    child.memory.env_start = parent.memory.env_start;
    child.memory.env_end = parent.memory.env_end;
}

#[expect(dead_code, reason = "argv/envp copy-in for execve")]
fn range_in_task_memory(task: &TaskStruct, addr: u32, len: usize) -> bool {
    if len == 0 {
        return true;
    }

    let start = addr as u64;
    let Some(end) = start.checked_add(len as u64) else {
        return false;
    };
    if end > (u32::MAX as u64) + 1 {
        return false;
    }

    let in_region = |base: u32, size: u32| {
        let region_end = base as u64 + size as u64;
        start >= base as u64 && end <= region_end
    };
    let memory = &task.memory;

    in_region(memory.code_base, memory.code_size)
        || in_region(memory.data_base, memory.data_size)
        || in_region(memory.bss_base, memory.bss_size)
        || (start >= memory.stack_limit as u64 && end <= memory.stack_base as u64)
        || (start >= memory.heap_base as u64 && end <= memory.heap_brk as u64)
        || memory
            .vmas
            .iter()
            .any(|vma| vma.used && in_region(vma.base, vma.size))
}

#[expect(dead_code, reason = "argv/envp copy-in for execve")]
pub(crate) unsafe fn copy_from_process(task: &TaskStruct, src: u32, dst: &mut [u8]) -> KResult<()> {
    unsafe {
        if !range_in_task_memory(task, src, dst.len()) {
            return Err(KernelError::Fault);
        }

        let old_cr3 = x86::read_cr3();
        let interrupts = x86::read_eflags();
        x86::disable_interrupts();
        x86::write_cr3(task.context.cr3);

        let mut copied = 0usize;
        while copied < dst.len() {
            let address = src.wrapping_add(copied as u32);
            if paging::virt_to_phys(address).is_none() {
                x86::write_cr3(old_cr3);
                x86::restore_eflags(interrupts);
                return Err(KernelError::Fault);
            }

            let page_offset = (address as usize) & (paging::PAGE_SIZE - 1);
            let chunk = (paging::PAGE_SIZE - page_offset).min(dst.len() - copied);
            core::ptr::copy_nonoverlapping(address as *const u8, dst[copied..].as_mut_ptr(), chunk);
            copied += chunk;
        }

        x86::write_cr3(old_cr3);
        x86::restore_eflags(interrupts);
        Ok(())
    }
}

#[expect(dead_code, reason = "argv/envp copy-in for execve")]
pub(crate) unsafe fn read_process_u32(task: &TaskStruct, address: u32) -> KResult<u32> {
    unsafe {
        let mut value = [0u8; core::mem::size_of::<u32>()];
        copy_from_process(task, address, &mut value)?;
        Ok(u32::from_ne_bytes(value))
    }
}

#[expect(dead_code, reason = "argv/envp copy-in for execve")]
pub(crate) unsafe fn copy_c_string_from_process(
    task: &TaskStruct,
    address: u32,
    output: &mut [u8],
) -> KResult<usize> {
    for (ndx, pbyte) in output.iter_mut().enumerate() {
        let value = {
            let mut buffer = [0u8; 1];
            (unsafe {
                copy_from_process(
                    task,
                    address.checked_add(ndx as u32).ok_or(KernelError::Fault)?,
                    &mut buffer,
                )
            })?;
            buffer[0]
        };
        if value == 0 {
            return Ok(ndx);
        }
        *pbyte = value;
    }
    Err(KernelError::TooBig)
}

#[expect(dead_code, reason = "argv/envp copy-in for execve")]
pub(crate) unsafe fn copy_envp_from_process(
    task: &TaskStruct,
    envp: u32,
    output: &mut [u8],
) -> KResult<usize> {
    if envp == 0 {
        return Ok(0);
    }

    let mut output_len = 0usize;
    for index in 0..ENV_MAX_ENTRIES {
        let pointer_address = envp
            .checked_add((index * core::mem::size_of::<u32>()) as u32)
            .ok_or(KernelError::Fault)?;
        let entry = unsafe { read_process_u32(task, pointer_address)? };
        if entry == 0 {
            return Ok(output_len);
        }

        let remaining = output.len().saturating_sub(output_len);
        if remaining == 0 {
            return Err(KernelError::TooBig);
        }
        let copied = unsafe { copy_c_string_from_process(task, entry, &mut output[output_len..])? };
        if copied == 0 || !output[output_len..output_len + copied].contains(&b'=') {
            return Err(KernelError::Inval);
        }
        output_len += copied + 1;
    }

    Err(KernelError::TooBig)
}

pub(crate) unsafe fn init_default_environment(task: &mut TaskStruct) -> KResult<()> {
    let entries: [&[u8]; 5] = [
        b"USER=root",
        b"HOME=/",
        b"PATH=/bin:/usr/bin",
        b"PWD=/",
        b"SHELL=/bin/mysh",
    ];
    let pointer_count = entries.len() + 1;
    let pointers_size = pointer_count * core::mem::size_of::<u32>();
    let strings_size: usize = entries.iter().map(|entry| entry.len() + 1).sum();
    if pointers_size + strings_size > ENV_MAX_SIZE {
        return Err(KernelError::TooBig);
    }

    let strings_start = ENV_BASE + pointers_size as u32;
    let mut string_offset = 0u32;

    for (index, entry) in entries.iter().enumerate() {
        let pointer = ENV_BASE + (index * core::mem::size_of::<u32>()) as u32;
        unsafe {
            (pointer as *mut u32).write(strings_start + string_offset);
            core::ptr::copy_nonoverlapping(
                entry.as_ptr(),
                (strings_start + string_offset) as *mut u8,
                entry.len(),
            );
            ((strings_start + string_offset) as *mut u8)
                .add(entry.len())
                .write(0);
        }
        string_offset += entry.len() as u32 + 1;
    }

    // envp NULL terminator
    let terminator = ENV_BASE + (entries.len() * core::mem::size_of::<u32>()) as u32;
    unsafe { (terminator as *mut u32).write(0) };

    task.env_start = ENV_BASE;
    task.memory.env_start = ENV_BASE;
    task.memory.env_end = strings_start + string_offset;
    Ok(())
}
