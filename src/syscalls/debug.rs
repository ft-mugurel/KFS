use kfs_abi::DebugOp;

use super::args::{UserAddr, UserSlice, caller, syscall_define};
use crate::{
    dump::{self, DEFAULT_TRACE_FRAMES, DumpStackOptions},
    error::{KResult, KernelError},
    interrupts::request_reboot,
    log::{KernelLogLevel, set_log_level},
    sched, security,
    smp::ipi::request_shutdown,
    vga::text_mod::{
        Color, ColorCode, active_screen_index, change_color, clear, print_fmt_on, print_str_on,
        switch_screen,
    },
};

syscall_define! {
    pub fn syscall_debug(op_raw: u32, arg1: u32, arg2: u32, arg3: u32, arg4: u32) {
        let op = DebugOp::from_u32(op_raw).ok_or(KernelError::Inval)?;
        debug_op(op, arg1, arg2, arg3, arg4)
    }
}

fn debug_op(op: DebugOp, arg1: u32, arg2: u32, arg3: u32, arg4: u32) -> KResult<u32> {
    // A copy: a reference into the task would alias PROCESS_TABLE, which
    // `Ps` locks.
    let cred = caller()?.credentials;

    let requires_root = !matches!(
        op,
        DebugOp::Clear
            | DebugOp::Users
            | DebugOp::Color
            | DebugOp::Screen
            | DebugOp::UserAdd
            | DebugOp::Su
            | DebugOp::Ps
            | DebugOp::Dmesg
            | DebugOp::Klog
    );
    if requires_root && !cred.is_root() {
        return Err(KernelError::Perm);
    }

    let screen = unsafe { sched::current().as_ref() }
        .and_then(|t| t.fd_tbl[1])
        .and_then(crate::fs::get_open_file)
        .map(|f| unsafe { &*f.node })
        .filter(|node| node.node_type == crate::fs::VfsNodeType::CharDevice)
        .map_or_else(active_screen_index, |node| node.inode as usize);

    match op {
        DebugOp::MemDump => {
            let addr = arg1;
            let len = if arg2 == 0 {
                64
            } else {
                (arg2 as usize).min(512)
            };
            dump::dump_virtual_memory(addr, len, |args| print_fmt_on(screen, args));
        }
        DebugOp::Pte => {
            let addr = arg1;
            dump::debug_page_entry(addr, |args| print_fmt_on(screen, args));
        }
        DebugOp::MemTest => {
            let name = if arg1 != 0 && arg2 != 0 && arg2 <= 64 {
                Some(UserSlice::new(UserAddr(arg1), arg2 as usize)?)
            } else {
                None
            };
            let test_name = name.as_ref().map_or("", |name| {
                core::str::from_utf8(name.as_bytes()).unwrap_or("")
            });
            dump::run_memtest(test_name, |args| print_fmt_on(screen, args));
        }
        DebugOp::Stack => {
            let words = if arg1 == 0 {
                16
            } else {
                (arg1 as usize).min(64)
            };
            let options = DumpStackOptions {
                words,
                frames: DEFAULT_TRACE_FRAMES,
                print_stack_values: true,
                walk_frames: true,
                scan_stack: true,
            };
            dump::dump_stack_with_options(options, |args| print_fmt_on(screen, &args));
        }
        DebugOp::Layout => {
            print_fmt_on(
                screen,
                &format_args!(
                    "Kernel Base: {:#x}, High Start: {:#x}\n",
                    0x0010_0000u32, 0xC000_0000u32
                ),
            );
        }
        DebugOp::Crash => {
            // SAFETY: none; this op exists to fault on purpose (root only) so
            // the page-fault path can be tested. The address is unmapped.
            unsafe { core::ptr::read_volatile(0xdeadbeef as *const u32) };
        }
        DebugOp::LogLevel => {
            let level = match arg1 {
                0 => KernelLogLevel::Emerg,
                1 => KernelLogLevel::Alert,
                2 => KernelLogLevel::Crit,
                3 => KernelLogLevel::Err,
                4 => KernelLogLevel::Warning,
                5 => KernelLogLevel::Notice,
                6 => KernelLogLevel::Info,
                7 => KernelLogLevel::Debug,
                _ => return Err(KernelError::Inval),
            };
            set_log_level(level);
            print_str_on(screen, "Log level updated\n");
        }
        DebugOp::Color => {
            let fg = Color::from_u8(arg1 as u8);
            let bg = Color::from_u8(arg2 as u8);
            change_color(ColorCode::new(fg, bg));
            print_str_on(screen, "Color updated\n");
        }
        DebugOp::Screen => {
            if arg1 >= crate::startup_config::vga::VIRTUAL_SCREENS as u32 {
                return Err(KernelError::Inval);
            }
            switch_screen(arg1 as usize);
        }
        DebugOp::Reboot => {
            unsafe { request_reboot() };
        }
        DebugOp::Shutdown => {
            unsafe { request_shutdown() };
        }
        DebugOp::Initcalls => {
            if let Some((start, end, pages)) = crate::initcall::freed_memory_info() {
                print_fmt_on(
                    screen,
                    &format_args!(
                        "Init memory reclaimed: {:#x} - {:#x} ({} pages)\n",
                        start, end, pages
                    ),
                );
            }
            let history = crate::initcall::boot_history();
            print_fmt_on(
                screen,
                &format_args!("Boot initcalls executed: {}\n", history.len()),
            );
            for record in history {
                let lvl_name = crate::initcall::InitcallLevel::from_u8(record.level)
                    .map(|l| l.as_str())
                    .unwrap_or("unknown");
                print_fmt_on(
                    screen,
                    &format_args!(
                        "  [{}] {:<32} -> ret={}\n",
                        lvl_name, record.name, record.result
                    ),
                );
            }
        }
        DebugOp::Clear => {
            clear(screen);
        }
        DebugOp::Users => {
            print_str_on(screen, " UID  | GID  | USERNAME\n");
            print_str_on(screen, "------+------+-----------------\n");
            let mut count = 0usize;
            security::for_each_account(|name, uid, gid| {
                count += 1;
                let name_str = core::str::from_utf8(name).unwrap_or("<invalid utf-8>");
                print_fmt_on(
                    screen,
                    &format_args!(" {:<4} | {:<4} | {}\n", uid, gid, name_str),
                );
            });
            if count == 0 {
                print_str_on(screen, "No user accounts found.\n");
            }
        }
        DebugOp::Su => {
            let uname = UserSlice::new(UserAddr(arg1), arg2 as usize)?;
            let uname = uname.as_bytes();
            let target = if uname == b"root" {
                crate::sched::Credentials::root()
            } else {
                security::credentials_for_user(uname).ok_or(KernelError::NoEnt)?
            };
            unsafe { security::set_current_credentials(target) };
        }
        DebugOp::UserAdd => {
            let uname = UserSlice::new(UserAddr(arg1), arg2 as usize)?;
            let pass = UserSlice::new(UserAddr(arg3), arg4 as usize)?;
            let (uname, pass) = (uname.as_bytes(), pass.as_bytes());
            if !cred.is_root() {
                let mut cur_buf = [0u8; 64];
                let is_self = security::username_for_uid(cred.uid, &mut cur_buf)
                    .is_some_and(|len| uname == &cur_buf[..len]);
                if !is_self {
                    return Err(KernelError::Perm);
                }
            }
            let user_id = if uname == b"root" {
                0
            } else {
                security::get_account_uid(uname)
                    .unwrap_or_else(|| security::next_user_id().unwrap_or(1000))
            };
            let group_id = user_id;
            let installed =
                security::create_account_record(uname, user_id, group_id, pass, 100_000)
                    .map(security::install_or_update_account)
                    .unwrap_or(false);
            if !(installed && security::ensure_passwd_user(uname, user_id, group_id)) {
                return Err(KernelError::Inval);
            }
            persist_security_files()?;
        }
        DebugOp::UserDel => {
            let username = UserSlice::new(UserAddr(arg1), arg2 as usize)?;
            let username = username.as_bytes();
            if username == b"root" {
                return Err(KernelError::Perm);
            }
            let removed_account = security::remove_account(username);
            let removed_passwd = security::remove_passwd(username);
            if !(removed_account || removed_passwd) {
                return Err(KernelError::NoEnt);
            }
            persist_security_files()?;
        }
        DebugOp::Ps => {
            print_str_on(screen, " PID | UID  | Parent PID | Exit Code  | State\n");
            for task in sched::PROCESS_TABLE.lock().iter().flatten() {
                let object =
                    crate::security::SecurityObject::Process { owner_uid: task.credentials.uid };
                if crate::security::check(&cred, &object, crate::security::Operation::Inspect)
                    == crate::security::Decision::Deny
                {
                    continue;
                }
                print_fmt_on(screen, &format_args!(" {:<3} |", task.pid));
                print_fmt_on(screen, &format_args!(" {:<4} |", task.credentials.uid));
                print_fmt_on(screen, &format_args!(" {:<10} |", task.family.parent_pid));
                print_fmt_on(
                    screen,
                    &format_args!(" {:<10} |", task.exit_code.unwrap_or(0)),
                );
                print_fmt_on(screen, &format_args!(" {:<20}\n", task.state));
            }
        }
        DebugOp::Dmesg => {
            let current_head = crate::log::klog::head();
            let mut offset = current_head.saturating_sub(crate::log::klog::KLOG_BUFFER_SIZE);
            let mut buf = [0u8; 512];
            while offset < current_head {
                let n = crate::log::klog::read_range(offset, &mut buf);
                if n == 0 {
                    break;
                }
                let slice = &buf[..n];
                let mut start = 0;
                while start < n {
                    match core::str::from_utf8(&slice[start..]) {
                        Ok(valid_str) => {
                            print_str_on(screen, valid_str);
                            break;
                        }
                        Err(e) => {
                            let valid_up_to = e.valid_up_to();
                            if valid_up_to > 0 {
                                if let Ok(valid_str) =
                                    core::str::from_utf8(&slice[start..start + valid_up_to])
                                {
                                    print_str_on(screen, valid_str);
                                }
                                start += valid_up_to;
                            }
                            if let Some(err_len) = e.error_len() {
                                start += err_len;
                            } else {
                                break;
                            }
                        }
                    }
                }
                offset += n;
            }
            if arg1 == 1 {
                crate::log::klog::clear();
            }
        }
        DebugOp::Klog => {
            if arg1 != 0 && arg2 > 0 {
                let msg = UserSlice::new(UserAddr(arg1), (arg2 as usize).min(256))?;
                if let Ok(s) = core::str::from_utf8(msg.as_bytes()) {
                    crate::pr_info!("[user] {}", s);
                } else {
                    crate::pr_info!("[user] <non-utf8 message>");
                }
            } else {
                crate::pr_info!(
                    "Kernel log probe test (ticks={})",
                    crate::interrupts::timer::get_ticks()
                );
            }
            let _ = crate::log::file_logger::flush_to_file();
        }
        DebugOp::ConsoleScreen => match arg1 {
            0 => {
                let enabled = crate::log::is_direct_screen_output_enabled();
                print_fmt_on(
                    screen,
                    &format_args!(
                        "Kernel direct screen output: {}\n",
                        if enabled { "enabled" } else { "disabled" }
                    ),
                );
                return Ok(enabled as u32);
            }
            1 => {
                crate::log::set_direct_screen_output(true);
                print_str_on(screen, "Kernel direct screen output enabled\n");
            }
            2 => {
                crate::log::set_direct_screen_output(false);
                print_str_on(screen, "Kernel direct screen output disabled\n");
            }
            _ => return Err(KernelError::Inval),
        },
        DebugOp::PrintErrors => {
            unsafe { sched::print_errors() };
        }
    }
    Ok(0)
}

fn persist_security_files() -> KResult<()> {
    let (acc_ok, pwd_ok) = unsafe { (security::persist_accounts(), security::persist_passwd()) };
    if acc_ok && pwd_ok {
        Ok(())
    } else {
        Err(KernelError::IO)
    }
}
