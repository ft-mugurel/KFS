use super::read_write::valid_user_buffer;
use crate::{
    dump::{self, DEFAULT_TRACE_FRAMES, DumpStackOptions},
    error::KernelError,
    interrupts::request_reboot,
    printk::{KernelLogLevel, set_log_level},
    sched::{self, ContextFrame},
    security,
    smp::ipi::request_shutdown,
    vga::text_mod::{
        Color, ColorCode, active_screen_index, change_color, clear, print_fmt_on, print_str_on,
        switch_screen,
    },
};

#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DebugOp {
    MemDump = 0,  // arg1: vaddr, arg2: len
    Pte = 1,      // arg1: vaddr
    MemTest = 2,  // arg1: test_ptr, arg2: test_len (or 0 for default)
    Stack = 3,    // arg1: words
    Layout = 4,   // display kernel segment layout
    Crash = 5,    // intentional test crash
    LogLevel = 6, // arg1: level (0..7)
    Color = 7,    // arg1: fg (0..15), arg2: bg (0..15)
    Screen = 8,   // arg1: screen index (0..5)
    Reboot = 9,
    Shutdown = 10,
    Initcalls = 11,
    Clear = 12,
    Users = 13,
    UserAdd = 14,
    Su = 15,
    Ps = 16,
    Dmesg = 17,
    Klog = 18,
    ConsoleScreen = 19,
    UserDel = 20,
}

impl DebugOp {
    pub fn from_u32(val: u32) -> Option<Self> {
        match val {
            0 => Some(Self::MemDump),
            1 => Some(Self::Pte),
            2 => Some(Self::MemTest),
            3 => Some(Self::Stack),
            4 => Some(Self::Layout),
            5 => Some(Self::Crash),
            6 => Some(Self::LogLevel),
            7 => Some(Self::Color),
            8 => Some(Self::Screen),
            9 => Some(Self::Reboot),
            10 => Some(Self::Shutdown),
            11 => Some(Self::Initcalls),
            12 => Some(Self::Clear),
            13 => Some(Self::Users),
            14 => Some(Self::UserAdd),
            15 => Some(Self::Su),
            16 => Some(Self::Ps),
            17 => Some(Self::Dmesg),
            18 => Some(Self::Klog),
            19 => Some(Self::ConsoleScreen),
            20 => Some(Self::UserDel),
            _ => None,
        }
    }
}

pub unsafe fn syscall_debug(regs: *mut ContextFrame) {
    let op_raw = (*regs).arg1();
    let Some(op) = DebugOp::from_u32(op_raw) else {
        (*regs).set_return_error(KernelError::EINVAL);
        return;
    };

    let is_root = unsafe {
        sched::current_cred()
            .as_ref()
            .map_or(false, |c| c.is_root())
    };

    let requires_root = match op {
        DebugOp::Clear
        | DebugOp::Users
        | DebugOp::Color
        | DebugOp::Screen
        | DebugOp::UserAdd
        | DebugOp::Su
        | DebugOp::Ps
        | DebugOp::Dmesg
        | DebugOp::Klog => false,
        _ => true,
    };

    if requires_root && !is_root {
        (*regs).set_return_error(KernelError::EPERM);
        return;
    }

    let arg1 = (*regs).arg2();
    let arg2 = (*regs).arg3();
    let screen = unsafe {
        sched::current()
            .as_ref()
            .and_then(|t| {
                t.fd_tbl[1].and_then(|gfd| {
                    crate::fs::get_open_file(gfd).and_then(|f| {
                        if (*f.node).node_type == crate::fs::VfsNodeType::CharDevice {
                            Some((*f.node).inode as usize)
                        } else {
                            None
                        }
                    })
                })
            })
            .unwrap_or_else(active_screen_index)
    };

    match op {
        DebugOp::MemDump => {
            let addr = arg1;
            let len = if arg2 == 0 {
                64
            } else {
                (arg2 as usize).min(512)
            };
            dump::dump_virtual_memory(addr, len, |args| print_fmt_on(screen, args));
            (*regs).set_return_value(0);
        }
        DebugOp::Pte => {
            let addr = arg1;
            dump::debug_page_entry(addr, |args| print_fmt_on(screen, args));
            (*regs).set_return_value(0);
        }
        DebugOp::MemTest => {
            let test_name = if arg1 != 0 && arg2 != 0 && arg2 <= 64 {
                let slice = core::slice::from_raw_parts(arg1 as *const u8, arg2 as usize);
                core::str::from_utf8(slice).unwrap_or("")
            } else {
                ""
            };
            dump::run_memtest(test_name, |args| print_fmt_on(screen, args));
            (*regs).set_return_value(0);
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
            (*regs).set_return_value(0);
        }
        DebugOp::Layout => {
            print_fmt_on(
                screen,
                &format_args!(
                    "Kernel Base: {:#x}, High Start: {:#x}\n",
                    0x0010_0000u32, 0xC000_0000u32
                ),
            );
            (*regs).set_return_value(0);
        }
        DebugOp::Crash => {
            core::ptr::read_volatile(0xdeadbeef as *const u32);
            (*regs).set_return_value(0);
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
                _ => {
                    (*regs).set_return_error(KernelError::EINVAL);
                    return;
                }
            };
            set_log_level(level);
            print_str_on(screen, "Log level updated\n");
            (*regs).set_return_value(0);
        }
        DebugOp::Color => {
            let fg = Color::from_u8(arg1 as u8);
            let bg = Color::from_u8(arg2 as u8);
            change_color(ColorCode::new(fg, bg));
            print_str_on(screen, "Color updated\n");
            (*regs).set_return_value(0);
        }
        DebugOp::Screen => {
            if arg1 < crate::startup_config::vga::VIRTUAL_SCREENS as u32 {
                switch_screen(arg1 as usize);
                (*regs).set_return_value(0);
            } else {
                (*regs).set_return_error(KernelError::EINVAL);
            }
        }
        DebugOp::Reboot => {
            request_reboot();
            (*regs).set_return_value(0);
        }
        DebugOp::Shutdown => {
            request_shutdown();
            (*regs).set_return_value(0);
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
            (*regs).set_return_value(0);
        }
        DebugOp::Clear => {
            clear(screen);
            (*regs).set_return_value(0);
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
            (*regs).set_return_value(0);
        }
        DebugOp::Su => {
            let user_ptr = (*regs).arg2() as *const u8;
            let user_len = (*regs).arg3() as usize;
            if valid_user_buffer(user_ptr as u32, user_len) {
                let uname = core::slice::from_raw_parts(user_ptr, user_len);
                if uname == b"root" {
                    security::set_current_credentials(crate::sched::Credentials::root());
                    (*regs).set_return_value(0);
                } else if let Some(cred) = security::credentials_for_user(uname) {
                    security::set_current_credentials(cred);
                    (*regs).set_return_value(0);
                } else {
                    (*regs).set_return_error(KernelError::ENOENT);
                }
            } else {
                (*regs).set_return_error(KernelError::EFAULT);
            }
        }
        DebugOp::UserAdd => {
            let user_ptr = (*regs).arg2() as *const u8;
            let user_len = (*regs).arg3() as usize;
            let pass_ptr = (*regs).arg4() as *const u8;
            let pass_len = (*regs).arg5() as usize;
            if valid_user_buffer(user_ptr as u32, user_len)
                && valid_user_buffer(pass_ptr as u32, pass_len)
            {
                let uname = core::slice::from_raw_parts(user_ptr, user_len);
                let pass = core::slice::from_raw_parts(pass_ptr, pass_len);
                if !is_root {
                    let current_uid = sched::current().as_ref().unwrap().credentials.uid;
                    let mut cur_buf = [0u8; 64];
                    let is_self =
                        if let Some(len) = security::username_for_uid(current_uid, &mut cur_buf) {
                            uname == &cur_buf[..len]
                        } else {
                            false
                        };
                    if !is_self {
                        (*regs).set_return_error(KernelError::EPERM);
                        return;
                    }
                }
                let is_root_target = uname == b"root";
                let user_id = if is_root_target {
                    0
                } else {
                    security::get_account_uid(uname)
                        .unwrap_or_else(|| security::next_user_id().unwrap_or(1000))
                };
                let group_id = user_id;
                let account =
                    security::create_account_record(uname, user_id, group_id, pass, 100_000);
                let installed = account
                    .map(|a| security::install_or_update_account(a))
                    .unwrap_or(false);
                if installed {
                    security::persist_accounts();
                    (*regs).set_return_value(0);
                } else {
                    (*regs).set_return_error(KernelError::EINVAL);
                }
            } else {
                (*regs).set_return_error(KernelError::EFAULT);
            }
        }
        DebugOp::UserDel => {
            let user_ptr = (*regs).arg2() as *const u8;
            let user_len = (*regs).arg3() as usize;
            if !valid_user_buffer(user_ptr as u32, user_len) {
                (*regs).set_return_error(KernelError::EFAULT);
            } else {
                let username = core::slice::from_raw_parts(user_ptr, user_len);
                if username == b"root" {
                    (*regs).set_return_error(KernelError::EPERM);
                } else if security::remove_account(username) {
                    if unsafe { security::persist_accounts() } {
                        (*regs).set_return_value(0);
                    } else {
                        (*regs).set_return_error(KernelError::EIO);
                    }
                } else {
                    (*regs).set_return_error(KernelError::ENOENT);
                }
            }
        }
        DebugOp::Ps => {
            print_str_on(screen, " PID | UID  | Parent PID | Exit Code  | State\n");
            let credentials = unsafe { sched::current().as_ref().unwrap().credentials };
            for process in sched::PROCESS_TABLE.lock().iter() {
                if let Some(task) = process {
                    let object = crate::security::SecurityObject::Process {
                        owner_uid: task.credentials.uid,
                    };
                    if crate::security::check(
                        &credentials,
                        &object,
                        crate::security::Operation::Inspect,
                    ) == crate::security::Decision::Deny
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
            (*regs).set_return_value(0);
        }
        DebugOp::Dmesg => {
            let current_head = crate::printk::klog::head();
            let mut offset = current_head.saturating_sub(crate::printk::klog::KLOG_BUFFER_SIZE);
            let mut buf = [0u8; 512];
            while offset < current_head {
                let n = crate::printk::klog::read_range(offset, &mut buf);
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
                crate::printk::klog::clear();
            }
            (*regs).set_return_value(0);
        }
        DebugOp::Klog => {
            let msg_ptr = arg1 as *const u8;
            let msg_len = arg2 as usize;
            if !msg_ptr.is_null() && msg_len > 0 {
                let to_read = msg_len.min(256);
                if valid_user_buffer(arg1, to_read) {
                    let mut buf = [0u8; 256];
                    for i in 0..to_read {
                        buf[i] = *msg_ptr.add(i);
                    }
                    if let Ok(s) = core::str::from_utf8(&buf[..to_read]) {
                        crate::pr_info!("[user] {}\n", s);
                    } else {
                        crate::pr_info!("[user] <non-utf8 message>\n");
                    }
                } else {
                    (*regs).set_return_error(KernelError::EFAULT);
                    return;
                }
            } else {
                crate::pr_info!(
                    "Kernel log probe test (ticks={})\n",
                    crate::interrupts::timer::get_ticks()
                );
            }
            let _ = crate::printk::file_logger::flush_to_file();
            (*regs).set_return_value(0);
        }
        DebugOp::ConsoleScreen => {
            match arg1 {
                0 => {
                    let enabled = crate::printk::is_direct_screen_output_enabled();
                    print_fmt_on(
                        screen,
                        &format_args!(
                            "Kernel direct screen output: {}\n",
                            if enabled { "enabled" } else { "disabled" }
                        ),
                    );
                    (*regs).set_return_value(if enabled { 1 } else { 0 });
                }
                1 => {
                    crate::printk::set_direct_screen_output(true);
                    print_str_on(screen, "Kernel direct screen output enabled\n");
                    (*regs).set_return_value(0);
                }
                2 => {
                    crate::printk::set_direct_screen_output(false);
                    print_str_on(screen, "Kernel direct screen output disabled\n");
                    (*regs).set_return_value(0);
                }
                _ => {
                    (*regs).set_return_error(KernelError::EINVAL);
                }
            }
        }
    }
}
