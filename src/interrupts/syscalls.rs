use crate::sched::scheduler::{CURRENT_PID, PROCESS_TABLE};
use crate::sched::task::ContextFrame;
unsafe extern "C" {
    fn isr_syscall();
}

const SYSCALL_VECTOR: u8 = 0x80;
const MAX_SYSCALL_NUMBER: usize = 256;

type SyscallHandler = fn(*mut ContextFrame);

fn syscall_exit(regs: *mut ContextFrame) {
    unsafe {
        let exit_code = (*regs).arg1();
        let current_pid = CURRENT_PID;
        crate::pr_info!("PID {} exited with code {}\n", current_pid, exit_code);

        let task = PROCESS_TABLE[current_pid].as_mut().unwrap();
        task.state = crate::sched::task::ProcessState::Zombie;
        let old_cr3 = task.context.cr3;
        let boot_cr3 = crate::paging::page_table::bootstrap_directory_phys_addr();

        crate::paging::physical::free_physical_page(old_cr3);
        crate::x86::write_cr3(boot_cr3);
        loop {
            core::arch::asm!("sti; hlt");
        }
    }
}

fn syscall_read(regs: *mut ContextFrame) {
    unsafe {
        let fd = (*regs).arg1();
        let buf_ptr = (*regs).arg2() as *const u8;
        let count = (*regs).arg3();
        crate::pr_debug!(
            "Syscall: read(fd={}, buf_ptr={:?}, count={})\n",
            fd,
            buf_ptr,
            count
        );
    }
}

fn syscall_write(regs: *mut ContextFrame) {
    unsafe {
        let fd = (*regs).arg1();
        let buf_ptr = (*regs).arg2() as *const u8;
        let count = (*regs).arg3();
        crate::pr_debug!(
            "Syscall: write(fd={}, buf_ptr={:?}, count={})\n",
            fd,
            buf_ptr,
            count
        );
        let current_pid = CURRENT_PID;
        let tty_id = PROCESS_TABLE[current_pid].as_ref().unwrap().tty_id;
        if fd == 1 {
            let slice = core::slice::from_raw_parts(buf_ptr, count as usize);
            if let Ok(s) = core::str::from_utf8(slice) {
                crate::vga::text_mod::out::print_on(tty_id, s);
            }
            (*regs).set_return_value(count);
        } else {
            crate::pr_warn!("Syscall write: We don't have fd's yet: {}\n", fd);
            (*regs).set_return_value(!0u32); // somewhat -1
        }
    }
}

fn syscall_wait(regs: *mut ContextFrame) {
    unsafe {
        let current_pid = CURRENT_PID;
        for i in 1..crate::sched::scheduler::MAX_PROCESSES {
            if let Some(ref mut child) = PROCESS_TABLE[i] {
                if child.family.parent_pid == current_pid as u32
                    && child.state == crate::sched::task::ProcessState::Zombie
                {
                    crate::pr_info!(
                        "Parent PID {} reaped Zombie PID {}\n",
                        current_pid,
                        child.pid
                    );
                    let child_pid = child.pid;
                    PROCESS_TABLE[i] = None;
                    (*regs).set_return_value(child_pid);
                    return;
                }
            }
        }
        (*regs).set_return_value(0);
    }
}

fn syscall_getuid(regs: *mut ContextFrame) {
    unsafe {
        let uid = PROCESS_TABLE[CURRENT_PID].as_ref().unwrap().uid;
        (*regs).set_return_value(uid);
    }
}

fn syscall_kill(regs: *mut ContextFrame) {
    unsafe {
        let target_pid = (*regs).arg1() as usize;
        let sig_num = (*regs).arg2() as u8;

        if target_pid >= crate::sched::scheduler::MAX_PROCESSES
            || PROCESS_TABLE[target_pid].is_none()
        {
            (*regs).set_return_value(!0u32);
            return;
        }

        let target_task = PROCESS_TABLE[target_pid].as_mut().unwrap();
        let queue = &mut target_task.signals;

        let next_head = (queue.head + 1) % crate::sched::task::SIGNAL_QUEUE_SIZE;
        if next_head != queue.tail {
            queue.pending[queue.head] = sig_num;
            queue.head = next_head;
            (*regs).set_return_value(0);
        } else {
            (*regs).set_return_value(!0u32);
        }
    }
}

fn syscall_signal(regs: *mut ContextFrame) {
    unsafe {
        let sig_num = (*regs).arg1() as usize;
        let handler_addr = (*regs).arg2(); // The memory address of the user's function
        let current_pid = CURRENT_PID;

        if sig_num >= crate::sched::task::MAX_SIGNALS {
            (*regs).set_return_value(!0u32);
            return;
        }

        // Register the user's function pointer in the task struct
        let task = PROCESS_TABLE[current_pid].as_mut().unwrap();
        task.signals.handlers[sig_num] = handler_addr;

        (*regs).set_return_value(0);
    }
}

fn syscall_sbrk(regs: *mut ContextFrame) {
    unsafe {
        let increment = (*regs).arg1() as i32;
        let current_pid = CURRENT_PID;
        let task = PROCESS_TABLE[current_pid].as_mut().unwrap();

        let old_brk = task.memory.heap_brk;

        if increment == 0 {
            (*regs).set_return_value(old_brk);
            return;
        }

        if increment < 0 {
            crate::pr_warn!("Shrinking the heap is not yet supported.\n");
            (*regs).set_return_value(!0u32); // Return -1
            return;
        }

        let new_brk = old_brk + increment as u32;
        let old_page_end = (old_brk + 4095) & !4095;

        if new_brk > old_page_end {
            let bytes_to_allocate = new_brk - old_page_end;
            let pages_to_allocate = (bytes_to_allocate + 4095) / 4096;

            for i in 0..pages_to_allocate {
                let vaddr = old_page_end + (i * 4096);

                if let Some(phys_frame) = crate::paging::physical::alloc_physical_page() {
                    let flags = crate::paging::page_table::PAGE_PRESENT
                        | crate::paging::page_table::PAGE_WRITABLE
                        | crate::paging::page_table::PAGE_USER;

                    crate::paging::page_table::map_page(vaddr, phys_frame, flags).unwrap();
                } else {
                    crate::pr_warn!("Out of physical memory for sbrk!\n");
                    (*regs).set_return_value(!0u32);
                    return;
                }
            }
        }

        task.memory.heap_brk = new_brk;
        (*regs).set_return_value(old_brk);
    }
}

fn syscall_sleep(regs: *mut ContextFrame) {
    unsafe {
        let sleep_ms = (*regs).arg1();
        let current_pid = CURRENT_PID;
        let ticks_to_wait =
            (sleep_ms as u64) * (crate::startup_config::power::CONFIG_HZ as u64) / 1000;
        let wakeup_tick = crate::interrupts::timer::get_ticks() + ticks_to_wait;
        let task = PROCESS_TABLE[current_pid].as_mut().unwrap();
        task.wakeup_time = wakeup_tick;
        task.state = crate::sched::task::ProcessState::Sleeping;

        while PROCESS_TABLE[current_pid].as_ref().unwrap().state
            == crate::sched::task::ProcessState::Sleeping
        {
            core::arch::asm!("sti; hlt");
        }
    }
}

static mut SYSCALL_ENTRIES: [Option<SyscallHandler>; MAX_SYSCALL_NUMBER] = {
    let mut table = [None; MAX_SYSCALL_NUMBER];
    table[1] = Some(syscall_exit as SyscallHandler);
    table[3] = Some(syscall_read as SyscallHandler);
    table[4] = Some(syscall_write as SyscallHandler);
    table[7] = Some(syscall_wait as SyscallHandler);
    table[24] = Some(syscall_getuid as SyscallHandler);
    table[37] = Some(syscall_kill as SyscallHandler);
    table[45] = Some(syscall_sbrk as SyscallHandler);
    table[48] = Some(syscall_signal as SyscallHandler);
    table[162] = Some(syscall_sleep as SyscallHandler);

    table
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_dispatcher(regs: *mut ContextFrame) {
    let syscall_no = unsafe { (*regs).eax } as usize;
    if syscall_no < MAX_SYSCALL_NUMBER {
        if let Some(handler) = SYSCALL_ENTRIES[syscall_no] {
            handler(regs);
            return;
        }
    }

    crate::pr_warn!("Unknown or unimplemented syscall number: {}\n", syscall_no);
    unsafe { (*regs).eax = (!0u32) - 38 + 1 };
}

pub fn init_syscalls() {
    crate::interrupts::idt::register_user_interrupt_handler(SYSCALL_VECTOR, isr_syscall);
}
