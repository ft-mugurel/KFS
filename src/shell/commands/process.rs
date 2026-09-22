use core::str;
use crate::pr_info;
use crate::pr_warn;
use crate::sched;
use crate::signals::{self, Signal};
use crate::test;
use crate::shell::init::{print, print_fmt};
use super::parse::parse_usize;

#[inline(always)]
pub(crate) fn command_wait() {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("eax") 7, // syscall number for wait
            options(nostack, nomem),
        );
        let status: i32;
        core::arch::asm!(
            "mov {}, eax",
            out(reg) status,
            options(nostack, nomem),
        );
        if status >= 0 {
            print_fmt(&format_args!("Reaped child process with PID {}.\n", status));
        } else {
            print("No child processes to wait for.\n");
        }
    }
}

#[inline(always)]
pub(crate) fn command_signal(mut parts: str::SplitWhitespace<'_>) {
    let Some(sig_str) = parts.next() else {
        print("usage: signal <signum|signal_name>\n");
        return;
    };

    let sig = if let Ok(num) = sig_str.parse::<u8>() {
        Signal::from_u8(num)
    } else {
        Signal::from_name(sig_str)
    };

    let Some(signal) = sig else {
        print("invalid signal\n");
        return;
    };

    unsafe { signals::send_signal(signal) };
}

#[inline(always)]
pub(crate) fn command_kill(mut parts: str::SplitWhitespace<'_>) {
    let Some(pid_str) = parts.next() else {
        print("usage: kill <pid> <signal>\n");
        return;
    };
    let Some(sig_str) = parts.next() else {
        print("usage: kill <pid> <signal>\n");
        return;
    };

    let pid = parse_usize(pid_str).unwrap_or(0);
    let sig = parse_usize(sig_str).unwrap_or(0);

    unsafe {
        let mut result: u32;
        core::arch::asm!(
            "int 0x80",
            in("eax") 37,       // sys_kill
            in("ebx") pid,
            in("ecx") sig,
            lateout("eax") result,
            options(nostack, nomem),
        );

        if result == 0 {
            print("Signal queued successfully.\n");
        } else {
            print("Failed to send signal. Invalid PID?\n");
        }
    }
}

#[inline(always)]
pub(crate) fn command_spawn() {
    print("Spawning user process...\n");
    unsafe {
        match sched::create_user_process(test::most_syscalls_we_have_probably, 1024) {
            Ok(pid) => {
                pr_info!("[SPAWN] shell: create_user_process succeeded\n");
                print_fmt(&format_args!(
                    "Spawned user process with PID {} and 1024 bytes.\n",
                    pid
                ));
            }
            Err(e) => {
                pr_warn!("[SPAWN] shell: create_user_process failed\n");
                print_fmt(&format_args!("Failed to spawn user process: {:?}\n", e));
            }
        }
    }
}

#[inline(always)]
pub(crate) fn command_ps() {
    print(" PID | UID  | Parent PID | Exit Code  | State\n");
    let credentials = unsafe { sched::current().as_ref().unwrap().credentials };
    for process in sched::PROCESS_TABLE.lock().iter() {
        if let Some(task) = process {
            let object =
                crate::security::SecurityObject::Process { owner_uid: task.credentials.uid };
            if crate::security::check(&credentials, &object, crate::security::Operation::Inspect)
                == crate::security::Decision::Deny
            {
                continue;
            }
            print_fmt(&format_args!(" {:<3} |", task.pid));
            print_fmt(&format_args!(" {:<4} |", task.credentials.uid));
            print_fmt(&format_args!(" {:<10} |", task.family.parent_pid));
            print_fmt(&format_args!(" {:<10} |", task.exit_code.unwrap_or(0)));
            print_fmt(&format_args!(" {:<20}\n", task.state));
        }
    }
}
