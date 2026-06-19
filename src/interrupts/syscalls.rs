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
                // CRITICAL: Route the text strictly to the caller's TTY!
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
    /* unsafe {
        // let pid = (*regs).arg1();
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
                    PROCESS_TABLE[i] = None;
                    (*regs).set_return_value(child.pid);
                }
            }
        }
        (*regs).set_return_value(0); // No zombie children found
    } */
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
