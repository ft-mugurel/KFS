use crate::io::{
    parse_dec, parse_hex, print_bytes, print_dec, print_dec_width, print_str, read_line_raw,
};
use crate::syscall::{
    DebugOp, LinuxDirent, O_CREAT, O_TRUNC, O_WRONLY, S_IFBLK, S_IFCHR, S_IFDIR, S_IFIFO, S_IFLNK,
    S_IFMT, S_IFSOCK, Stat, sys_chdir, sys_close, sys_debug, sys_debug4, sys_fork, sys_getcwd,
    sys_getdents, sys_getuid, sys_getusername, sys_kill, sys_login, sys_lstat, sys_mknod,
    sys_nanosleep, sys_open, sys_read, sys_readlink, sys_unlink, sys_wait, sys_write,
};

pub const MAX_INPUT_LEN: usize = 256;

pub const COMMANDS: &[&str] = &[
    "cat",
    "cd",
    "chaos",
    "clear",
    "color",
    "crash",
    "dmesg",
    "echo",
    "exit",
    "help",
    "initcalls",
    "kill",
    "klog",
    "layout",
    "login",
    "loglevel",
    "logout",
    "ls",
    "memdebug",
    "memdump",
    "memstat",
    "memtest",
    "mkdir",
    "passwd",
    "print_errors",
    "ps",
    "pte",
    "pwd",
    "readlink",
    "reboot",
    "rm",
    "screen",
    "shutdown",
    "spawn",
    "stack",
    "su",
    "sysinfo",
    "touch",
    "truncate",
    "useradd",
    "userdel",
    "users",
    "wait",
    "whoami",
];

pub struct History {
    pub entries: [[u8; MAX_INPUT_LEN]; 16],
    pub count: usize,
    pub offset: usize,
    pub saved_input: [u8; MAX_INPUT_LEN],
    pub saved_len: usize,
}

impl History {
    pub const fn new() -> Self {
        Self {
            entries: [[0; MAX_INPUT_LEN]; 16],
            count: 0,
            offset: 0,
            saved_input: [0; MAX_INPUT_LEN],
            saved_len: 0,
        }
    }

    pub fn add(&mut self, command: &str) {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            self.offset = 0;
            return;
        }

        if self.count > 0 {
            let last_idx = (self.count - 1) % 16;
            let last_len = self.entries[last_idx]
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(MAX_INPUT_LEN);
            if let Ok(last_str) = core::str::from_utf8(&self.entries[last_idx][..last_len])
                && last_str == trimmed
            {
                self.offset = 0;
                return;
            }
        }

        let idx = self.count % 16;
        self.entries[idx].fill(0);
        let bytes = trimmed.as_bytes();
        let copy_len = bytes.len().min(MAX_INPUT_LEN - 1);
        self.entries[idx][..copy_len].copy_from_slice(&bytes[..copy_len]);
        self.count += 1;
        self.offset = 0;
    }
}

pub fn longest_common_prefix(partial: &str) -> (&'static str, usize) {
    let mut matches = COMMANDS
        .iter()
        .copied()
        .filter(|cmd| cmd.starts_with(partial));

    let Some(first) = matches.next() else {
        return ("", 0);
    };

    let mut count = 1;
    let mut prefix_len = first.len();

    for cmd in matches {
        count += 1;
        let mut common_len = 0;
        for (b1, b2) in first[..prefix_len].bytes().zip(cmd.bytes()) {
            if b1 == b2 {
                common_len += 1;
            } else {
                break;
            }
        }
        prefix_len = common_len;
    }

    (&first[..prefix_len], count)
}

fn echo_to_file(text: &str, path: &str) {
    let mut null_path = [0u8; 128];
    if path.is_empty() || path.len() >= null_path.len() {
        print_str("Usage: echo <text> > <path>\n");
        return;
    }
    null_path[..path.len()].copy_from_slice(path.as_bytes());
    let fd = sys_open(
        &null_path[..=path.len()],
        O_WRONLY | O_CREAT | O_TRUNC,
        0o644,
    );
    if fd < 0 {
        print_str("echo: cannot open ");
        print_str(path);
        print_str("\n");
        return;
    }
    let mut line = [0u8; MAX_INPUT_LEN + 1];
    let len = text.len().min(MAX_INPUT_LEN);
    line[..len].copy_from_slice(&text.as_bytes()[..len]);
    line[len] = b'\n';
    if sys_write(fd as usize, &line[..=len]) < 0 {
        print_str("echo: write failed\n");
    }
    sys_close(fd as usize);
}

fn dump_file(path: &str) {
    let mut null_term_path = [0u8; 128];
    if path.len() >= null_term_path.len() {
        print_str("Path too long\n");
        return;
    }
    null_term_path[..path.len()].copy_from_slice(path.as_bytes());
    null_term_path[path.len()] = 0;

    let fd = sys_open(&null_term_path[..=path.len()], 0, 0);
    if fd < 0 {
        print_str("Error opening file: ");
        print_str(path);
        print_str("\n");
        return;
    }

    let fd = fd as usize;
    let mut buf = [0u8; 256];
    loop {
        let n = sys_read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        sys_write(1, &buf[..n as usize]);
    }
    sys_close(fd);
}

const LS_PATH_MAX: usize = 128;
const NAME_MAX: usize = 256;

fn mode_string(mode: u32) -> [u8; 10] {
    let mut out = *b"----------";
    out[0] = match mode & S_IFMT {
        S_IFDIR => b'd',
        S_IFLNK => b'l',
        S_IFCHR => b'c',
        S_IFBLK => b'b',
        S_IFIFO => b'p',
        S_IFSOCK => b's',
        _ => b'-',
    };
    for i in 0..9 {
        if mode & (0o400 >> i) != 0 {
            out[i + 1] = b"rwx"[i % 3];
        }
    }
    // setuid, setgid and sticky share a column with the execute bit.
    for (bit, column, letter) in [(0o4000, 3, b's'), (0o2000, 6, b's'), (0o1000, 9, b't')] {
        if mode & bit != 0 {
            let executable = out[column] == b'x';
            out[column] = if executable { letter } else { letter.to_ascii_uppercase() };
        }
    }
    out
}

/// Reads as much of /etc/passwd as fits in `buf`; a cut-off last line only
/// means that user shows up as a number.
fn read_passwd(buf: &mut [u8]) -> usize {
    let fd = sys_open(b"/etc/passwd\0", 0, 0);
    if fd < 0 {
        return 0;
    }
    let fd = fd as usize;
    let mut len = 0;
    while len < buf.len() {
        let n = sys_read(fd, &mut buf[len..]);
        if n <= 0 {
            break;
        }
        len += n as usize;
    }
    sys_close(fd);
    len
}

fn owner_name(passwd: &[u8], uid: u32) -> Option<&str> {
    core::str::from_utf8(passwd).ok()?.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let id = fields.nth(1)?;
        (id.parse() == Ok(uid)).then_some(name)
    })
}

/// One `ls -l` line. `path` is NUL terminated and locates the entry, `name`
/// is what gets displayed.
fn print_long_entry(path: &[u8], name: &str, stat: &Stat, passwd: &[u8]) {
    print_bytes(&mode_string(stat.st_mode));
    print_dec_width(stat.st_nlink as usize, 3);
    print_str(" ");
    match owner_name(passwd, stat.st_uid) {
        Some(owner) => {
            print_str(owner);
            for _ in owner.len()..8 {
                print_str(" ");
            }
        }
        None => print_dec_width(stat.st_uid as usize, 8),
    }
    print_dec_width(stat.st_gid as usize, 5);
    print_dec_width(stat.st_size as usize, 9);
    print_str(" ");
    print_str(name);
    if stat.st_mode & S_IFMT == S_IFLNK {
        let mut target = [0u8; NAME_MAX];
        let len = sys_readlink(path, &mut target);
        if len > 0 {
            print_str(" -> ");
            print_bytes(&target[..len as usize]);
        }
    }
    print_str("\n");
}

fn print_ls_access_error(path: &str) {
    print_str("ls: cannot access '");
    print_str(path);
    print_str("'\n");
}

fn command_ls<'a>(first: &'a str, rest: impl Iterator<Item = &'a str>) {
    let mut long = false;
    let mut path = "";
    for arg in core::iter::once(first).chain(rest) {
        if arg == "-l" {
            long = true;
        } else if arg.len() > 1 && arg.starts_with('-') {
            print_str("ls: invalid option '");
            print_str(arg);
            print_str("'\n");
            return;
        } else if path.is_empty() {
            path = arg;
        }
    }
    if path.is_empty() {
        path = ".";
    }

    let mut null_path = [0u8; LS_PATH_MAX];
    if path.len() >= null_path.len() {
        print_str("ls: path too long\n");
        return;
    }
    null_path[..path.len()].copy_from_slice(path.as_bytes());
    let null_path = &null_path[..=path.len()];

    let mut passwd = [0u8; 1024];
    let passwd_len = if long { read_passwd(&mut passwd) } else { 0 };
    let passwd = &passwd[..passwd_len];

    if long {
        let mut stat = Stat::default();
        if sys_lstat(null_path, &mut stat) < 0 {
            print_ls_access_error(path);
            return;
        }
        if stat.st_mode & S_IFMT != S_IFDIR {
            print_long_entry(null_path, path, &stat, passwd);
            return;
        }
    }

    let fd = sys_open(null_path, 0, 0);
    if fd < 0 {
        print_ls_access_error(path);
        return;
    }
    let fd = fd as usize;

    // "<path>/<name>\0", the path of the entry being listed.
    let mut entry_path = [0u8; LS_PATH_MAX + 1 + NAME_MAX + 1];
    entry_path[..path.len()].copy_from_slice(path.as_bytes());
    entry_path[path.len()] = b'/';
    let name_start = path.len() + 1;

    let mut buf = [0u8; 512];
    loop {
        let n = sys_getdents(fd, &mut buf);
        if n <= 0 {
            break;
        }
        let mut offset = 0;
        while offset < n as usize {
            let dirent = unsafe { &*(buf.as_ptr().add(offset) as *const LinuxDirent) };
            let reclen = dirent.d_reclen as usize;
            if reclen == 0 || offset + reclen > n as usize {
                break;
            }
            let name_ptr = unsafe { buf.as_ptr().add(offset + 10) };
            let max_name_len = reclen - 10;
            let mut name_len = 0;
            while name_len < max_name_len && unsafe { *name_ptr.add(name_len) } != 0 {
                name_len += 1;
            }
            if let Ok(name_str) =
                core::str::from_utf8(unsafe { core::slice::from_raw_parts(name_ptr, name_len) })
            {
                if long {
                    let name_end = name_start + name_str.len();
                    entry_path[name_start..name_end].copy_from_slice(name_str.as_bytes());
                    entry_path[name_end] = 0;
                    let entry = &entry_path[..=name_end];
                    let mut stat = Stat::default();
                    if sys_lstat(entry, &mut stat) < 0 {
                        print_ls_access_error(name_str);
                    } else {
                        print_long_entry(entry, name_str, &stat, passwd);
                    }
                } else {
                    print_str(name_str);
                    print_str("  ");
                }
            }
            offset += reclen;
        }
    }
    if !long {
        print_str("\n");
    }
    sys_close(fd);
}

fn print_help() {
    print_str("Available commands:\n");
    print_str("File and shell commands:\n");
    print_str("  help, clear, echo, pwd, cd, ls [-l], cat, touch, truncate, mkdir, rm, whoami\n");
    print_str("  readlink <path>, echo <text> > <path>\n");
    print_str("User management:\n");
    print_str("  login, logout, su, useradd, userdel, passwd, users\n");
    print_str("Process management:\n");
    print_str("  ps, spawn, wait, kill <pid>, chaos [procs] [rounds]\n");
    print_str("Kernel and diagnostics:\n");
    print_str("  memstat, memdebug, initcalls, sysinfo, dmesg [-c], print_errors\n");
    print_str("  klog [msg|console on/off/status], memdump <hex_addr>, pte <hex_addr>\n");
    print_str("Testing and debugging:\n");
    print_str("  memtest, stack [words], layout, crash\n");
    print_str("Display and logging:\n");
    print_str("  loglevel <0-7>, color <0-15>, screen <1-6>\n");
    print_str("System control:\n");
    print_str("  reboot, shutdown, exit\n");
}

#[inline(never)]
pub fn execute_command(line: &str, history: &mut History) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return true;
    }

    let mut parts = trimmed.split_whitespace();
    let cmd = match parts.next() {
        Some(c) => c,
        None => return true,
    };
    let arg = parts.next().unwrap_or("");

    match cmd {
        "help" => print_help(),
        "clear" => {
            print_str("\x1b[2J\x1b[H");
        }
        "print_errors" => {
            sys_debug(DebugOp::PrintErrors, 0, 0);
        }
        "klog" => {
            let rest = trimmed.strip_prefix("klog").unwrap_or("").trim_start();
            if rest == "console on" {
                if sys_getuid() != 0 {
                    print_str("permission denied\n");
                } else {
                    sys_debug(DebugOp::ConsoleScreen, 1, 0);
                }
            } else if rest == "console off" {
                if sys_getuid() != 0 {
                    print_str("permission denied\n");
                } else {
                    sys_debug(DebugOp::ConsoleScreen, 2, 0);
                }
            } else if rest == "console status" || rest == "console" {
                if sys_getuid() != 0 {
                    print_str("permission denied\n");
                } else {
                    sys_debug(DebugOp::ConsoleScreen, 0, 0);
                }
            } else if rest.is_empty() {
                sys_debug(DebugOp::Klog, 0, 0);
                print_str("Kernel log emitted and flushed to /var/log/kernel.log\n");
            } else {
                sys_debug(DebugOp::Klog, rest.as_ptr() as u32, rest.len() as u32);
                print_str("Kernel log emitted and flushed to /var/log/kernel.log\n");
            }
        }
        "echo" => {
            let rest = trimmed.strip_prefix("echo").unwrap_or("").trim_start();
            match rest.rsplit_once('>') {
                Some((text, path)) => echo_to_file(text.trim_end(), path.trim()),
                None => {
                    print_str(rest);
                    print_str("\n");
                }
            }
        }
        "readlink" => {
            let mut null_path = [0u8; 128];
            if arg.is_empty() || arg.len() >= null_path.len() {
                print_str("Usage: readlink <path>\n");
            } else {
                null_path[..arg.len()].copy_from_slice(arg.as_bytes());
                let mut target = [0u8; 256];
                let len = sys_readlink(&null_path[..=arg.len()], &mut target);
                match core::str::from_utf8(&target[..len.max(0) as usize]) {
                    Ok(target) if len > 0 => {
                        print_str(target);
                        print_str("\n");
                    }
                    _ => print_str("readlink: not a readable symbolic link\n"),
                }
            }
        }
        "pwd" => {
            let mut buf = [0u8; 128];
            let ret = sys_getcwd(&mut buf);
            if ret > 0 {
                let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                if let Ok(s) = core::str::from_utf8(&buf[..len]) {
                    print_str(s);
                    print_str("\n");
                }
            } else {
                print_str("/\n");
            }
        }
        "cd" => {
            let target = if arg.is_empty() { "/" } else { arg };
            let mut null_target = [0u8; 128];
            if target.len() >= null_target.len() {
                print_str("cd: path too long\n");
            } else {
                null_target[..target.len()].copy_from_slice(target.as_bytes());
                null_target[target.len()] = 0;
                if sys_chdir(&null_target[..=target.len()]) < 0 {
                    print_str("cd: no such file or directory\n");
                }
            }
        }
        "ls" => command_ls(arg, parts),
        "cat" => {
            if arg.is_empty() {
                print_str("Usage: cat <path>\n");
            } else {
                dump_file(arg);
            }
        }
        "touch" => {
            if arg.is_empty() {
                print_str("Usage: touch <path>\n");
            } else {
                let mut null_path = [0u8; 128];
                if arg.len() >= null_path.len() {
                    print_str("touch: path too long\n");
                } else {
                    null_path[..arg.len()].copy_from_slice(arg.as_bytes());
                    null_path[arg.len()] = 0;
                    let fd = sys_open(&null_path[..=arg.len()], 0x42, 420);
                    if fd >= 0 {
                        sys_close(fd as usize);
                    } else {
                        print_str("touch: failed to create file\n");
                    }
                }
            }
        }
        "truncate" => {
            let mut no_create = false;
            let mut target_path = "";
            let mut iter = trimmed
                .strip_prefix("truncate")
                .unwrap_or("")
                .split_whitespace();
            let mut size_unsupported = false;

            while let Some(tok) = iter.next() {
                if tok == "-c" || tok == "--no-create" {
                    no_create = true;
                } else if tok == "-s" {
                    if let Some(s) = iter.next() {
                        if s != "0" {
                            size_unsupported = true;
                        }
                    } else {
                        target_path = "";
                        break;
                    }
                } else if !tok.starts_with('-') {
                    target_path = tok;
                }
            }

            if size_unsupported {
                print_str("truncate: only size 0 is supported\n");
            } else if target_path.is_empty() {
                print_str("Usage: truncate [-c] [-s 0] <path>\n");
            } else {
                let mut null_path = [0u8; 128];
                if target_path.len() >= null_path.len() {
                    print_str("truncate: path too long\n");
                } else {
                    null_path[..target_path.len()].copy_from_slice(target_path.as_bytes());
                    null_path[target_path.len()] = 0;
                    let flags = if no_create {
                        O_WRONLY | O_TRUNC
                    } else {
                        O_WRONLY | O_CREAT | O_TRUNC
                    };
                    let fd = sys_open(&null_path[..=target_path.len()], flags, 0o644);
                    if fd >= 0 {
                        sys_close(fd as usize);
                    } else {
                        print_str("truncate: failed to truncate file\n");
                    }
                }
            }
        }
        "mkdir" => {
            if arg.is_empty() {
                print_str("Usage: mkdir <path>\n");
            } else {
                let mut null_path = [0u8; 128];
                if arg.len() >= null_path.len() {
                    print_str("mkdir: path too long\n");
                } else {
                    null_path[..arg.len()].copy_from_slice(arg.as_bytes());
                    null_path[arg.len()] = 0;
                    if sys_mknod(&null_path[..=arg.len()], 0x4000 | 0o755) < 0 {
                        print_str("mkdir: failed to create directory\n");
                    }
                }
            }
        }
        "rm" => {
            if arg.is_empty() {
                print_str("Usage: rm <path>\n");
            } else {
                let mut null_path = [0u8; 128];
                if arg.len() >= null_path.len() {
                    print_str("rm: path too long\n");
                } else {
                    null_path[..arg.len()].copy_from_slice(arg.as_bytes());
                    null_path[arg.len()] = 0;
                    if sys_unlink(&null_path[..=arg.len()]) < 0 {
                        print_str("rm: failed to remove file\n");
                    }
                }
            }
        }
        "whoami" => {
            let mut name_buf = [0u8; 64];
            let ret = sys_getusername(&mut name_buf);
            if ret > 0 {
                if let Ok(name) = core::str::from_utf8(&name_buf[..ret as usize]) {
                    print_str(name);
                    print_str("\n");
                }
            } else {
                let uid = sys_getuid();
                if uid == 0 {
                    print_str("root\n");
                } else {
                    print_str("uid ");
                    print_dec(uid as usize);
                    print_str("\n");
                }
            }
        }
        "login" => {
            print_str("Logout.\n");
            return false;
        }
        "logout" => {
            print_str("Logout.\n");
            return false;
        }
        "su" => {
            let target = if arg.is_empty() { "root" } else { arg };
            let uid = sys_getuid();
            if uid == 0
                && target != "root"
                && sys_debug4(
                    DebugOp::Su,
                    target.as_ptr() as u32,
                    target.len() as u32,
                    0,
                    0,
                ) < 0
            {
                print_str("user not found\n");
            } else {
                let mut pass_buf = [0u8; 64];
                let plen = read_line_raw("Password: ", &mut pass_buf, true, history);
                let password = &pass_buf[..plen];
                if sys_login(target.as_bytes(), password) != 0 {
                    print_str("su: Authentication failure\n");
                }
            }
        }
        "users" => {
            sys_debug(DebugOp::Users, 0, 0);
        }
        "sleep" => {
            if arg.is_empty() {
                print_str("usage: sleep <seconds>\n");
            } else {
                sys_nanosleep(parse_dec(arg).unwrap_or(1) as u32);
            }
        }
        "useradd" => {
            if arg.is_empty() {
                print_str("usage: useradd <name>\n");
            } else if sys_getuid() != 0 {
                print_str("permission denied\n");
            } else {
                let mut pass_buf = [0u8; 64];
                let plen = read_line_raw("New password: ", &mut pass_buf, true, history);
                let password = &pass_buf[..plen];
                if sys_debug4(
                    DebugOp::UserAdd,
                    arg.as_ptr() as u32,
                    arg.len() as u32,
                    password.as_ptr() as u32,
                    password.len() as u32,
                ) == 0
                {
                    print_str("User updated/added\n");
                } else {
                    print_str("User creation failed\n");
                }
            }
        }
        "userdel" => {
            if arg.is_empty() {
                print_str("usage: userdel <name>\n");
            } else if sys_getuid() != 0 {
                print_str("permission denied\n");
            } else if sys_debug(DebugOp::UserDel, arg.as_ptr() as u32, arg.len() as u32) == 0 {
                print_str("User deleted\n");
            } else {
                print_str("User deletion failed\n");
            }
        }
        "passwd" => {
            let mut cur_buf = [0u8; 64];
            let cur_ret = sys_getusername(&mut cur_buf);
            let cur_name = if cur_ret > 0 {
                core::str::from_utf8(&cur_buf[..cur_ret as usize]).unwrap_or("root")
            } else {
                "root"
            };
            let target = if arg.is_empty() { cur_name } else { arg };
            if sys_getuid() != 0 && target != cur_name {
                print_str("permission denied\n");
            } else {
                let mut pass_buf = [0u8; 64];
                let plen = read_line_raw("New password: ", &mut pass_buf, true, history);
                let password = &pass_buf[..plen];
                if sys_debug4(
                    DebugOp::UserAdd,
                    target.as_ptr() as u32,
                    target.len() as u32,
                    password.as_ptr() as u32,
                    password.len() as u32,
                ) == 0
                {
                    print_str("User updated/added\n");
                } else {
                    print_str("User creation failed\n");
                }
            }
        }
        "ps" => {
            sys_debug(DebugOp::Ps, 0, 0);
        }
        "spawn" => {
            let pid = sys_fork();
            if pid == 0 {
                print_str("Child process running\n");
                crate::syscall::sys_exit(0);
            } else if pid > 0 {
                print_str("Spawned child PID ");
                print_dec(pid as usize);
                print_str("\n");
            } else {
                print_str("Failed to spawn child process\n");
            }
        }
        "wait" => {
            let pid = sys_wait();
            if pid >= 0 {
                print_str("Reaped child PID ");
                print_dec(pid as usize);
                print_str("\n");
            } else {
                print_str("No child processes to wait for\n");
            }
        }
        "chaos" => {
            let rest = trimmed.strip_prefix("chaos").unwrap_or("").trim_start();
            crate::chaos::run_chaos_test(rest);
        }
        "kill" => {
            if let Some(pid) = parse_dec(arg) {
                if sys_kill(pid, 15) < 0 {
                    print_str("kill: failed to kill process\n");
                }
            } else {
                print_str("Usage: kill <pid>\n");
            }
        }
        "memstat" => dump_file("/sys/kernel/debug/memstat"),
        "memdebug" => dump_file("/sys/kernel/debug/memdebug"),
        "initcalls" => dump_file("/sys/kernel/debug/initcalls"),
        "sysinfo" => dump_file("/sys/kernel/debug/sysinfo"),
        "dmesg" => {
            let clear = if arg == "-c" { 1 } else { 0 };
            sys_debug(DebugOp::Dmesg, clear, 0);
        }
        "memdump" => {
            let addr = parse_hex(arg).unwrap_or(0xC010_0000);
            sys_debug(DebugOp::MemDump, addr, 64);
        }
        "pte" => {
            let addr = parse_hex(arg).unwrap_or(0xC010_0000);
            sys_debug(DebugOp::Pte, addr, 0);
        }
        "memtest" => {
            sys_debug(DebugOp::MemTest, 0, 0);
        }
        "stack" => {
            let words = parse_dec(arg).unwrap_or(16) as u32;
            sys_debug(DebugOp::Stack, words, 0);
        }
        "layout" => {
            sys_debug(DebugOp::Layout, 0, 0);
        }
        "divzero" => {
            #[allow(unconditional_panic)]
            let _ = 1 / 0;
        }
        "crash" => {
            sys_debug(DebugOp::Crash, 0, 0);
        }
        "loglevel" => {
            let lvl = parse_dec(arg).unwrap_or(7) as u32;
            sys_debug(DebugOp::LogLevel, lvl, 0);
        }
        "color" => {
            let col = parse_dec(arg).unwrap_or(15) as u32;
            sys_debug(DebugOp::Color, col, 0);
        }
        "screen" => {
            let scr = parse_dec(arg).unwrap_or(1) as u32;
            sys_debug(DebugOp::Screen, scr, 0);
        }
        "reboot" => {
            sys_debug(DebugOp::Reboot, 0, 0);
        }
        "shutdown" => {
            sys_debug(DebugOp::Shutdown, 0, 0);
        }
        "exit" => {
            print_str("Exiting shell...\n");
            return false;
        }
        unknown => {
            print_str("Unknown command: ");
            print_str(unknown);
            print_str(". Type 'help' for available commands.\n");
        }
    }

    true
}

pub fn run_shell(_user: &str, history: &mut History) {
    print_str("[+] KFS User-Space Shell (mysh) loaded in Ring 3\n");
    print_str("Type 'help' for a list of available commands.\n\n");

    let mut line_buffer = [0u8; MAX_INPUT_LEN];

    loop {
        let len = read_line_raw("mysh > ", &mut line_buffer, false, history);
        if len > 0
            && let Ok(line_str) = core::str::from_utf8(&line_buffer[..len])
        {
            history.add(line_str);
            let keep_running = execute_command(line_str, history);
            if !keep_running {
                break;
            }
        }
    }
}
