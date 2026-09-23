use core::cell::UnsafeCell;
use core::fmt;
use core::str;

use crate::{
    dump, fs,
    interrupts::{
        keyboard::{self, KeyCode, KeyEvent, Modifiers},
        request_reboot,
    },
    security,
    smp::ipi::request_shutdown,
    startup_config,
    vga::text_mod::{
        active_cursor_position, active_screen_accepts_input, clear, is_screen_active,
        print_char_on, print_fmt_on, print_str_on, scroll_view_to_bottom, set_cursor_movement_on,
        set_cursor_position_on, set_screen_accepts_input, switch_to_next_screen,
        switch_to_previous_screen, CursorMovement, VGA_WIDTH,
    },
};

use super::commands;

pub(crate) const PROMPT: &str = "mysh > ";
pub(crate) const MAX_INPUT_LEN: usize = startup_config::shell::MAX_INPUT_LEN;
const SCREEN_INDEX: usize = startup_config::shell::SCREEN_INDEX;
const COMMANDS: &[&str] = &[
    "help",
    "clear",
    "echo",
    "reboot",
    "shutdown",
    "screen",
    "loglevel",
    "color",
    "memstat",
    "memdebug",
    "memdump",
    "pte",
    "memtest",
    "stack",
    "crash",
    "layout",
    "signal",
    "spawn",
    "wait",
    "kill",
    "ps",
    "fs_test",
    "fs_tree",
    "devices",
    "storage_test",
    "mkdir",
    "mount",
    "umount",
    "useradd",
    "users",
    "login",
    "su",
    "passwd",
    "whoami",
    "logout",
    "initcalls",
];

pub(crate) struct ShellState {
    pub(crate) input: [u8; MAX_INPUT_LEN],
    pub(crate) idx: usize,
    pub(crate) len: usize,
    pub(crate) rendered_len: usize,
    pub(crate) initialized: bool,
    pub(crate) history: [[u8; MAX_INPUT_LEN]; 16],
    pub(crate) history_idx: usize,
    pub(crate) current_history_offset: usize,
    pub(crate) saved_input: [u8; MAX_INPUT_LEN],
    pub(crate) saved_len: usize,
    pub(crate) login_stage: u8,
    pub(crate) login_username: [u8; MAX_INPUT_LEN],
    pub(crate) pending_username: [u8; MAX_INPUT_LEN],
}

impl ShellState {
    const fn new() -> Self {
        Self {
            input: [0; MAX_INPUT_LEN],
            idx: 0,
            len: 0,
            rendered_len: 0,
            initialized: false,
            history: [[0; MAX_INPUT_LEN]; 16],
            history_idx: 0,
            current_history_offset: 0,
            saved_input: [0; MAX_INPUT_LEN],
            saved_len: 0,
            login_stage: 0,
            login_username: [0; MAX_INPUT_LEN],
            pending_username: [0; MAX_INPUT_LEN],
        }
    }

    pub(crate) fn clear_input(&mut self) {
        self.len = 0;
        self.idx = 0;
        self.rendered_len = 0;
    }

    fn push_char(&mut self, c: char) -> bool {
        if !c.is_ascii() || c.is_ascii_control() || self.len >= MAX_INPUT_LEN {
            return false;
        }
        if self.idx < self.len {
            self.input.copy_within(self.idx..self.len, self.idx + 1);
        }
        self.input[self.idx] = c as u8;
        self.idx += 1;
        self.len += 1;
        true
    }

    fn delete_char(&mut self) -> bool {
        if self.idx == 0 {
            return false;
        }
        self.input.copy_within(self.idx..self.len, self.idx - 1);
        self.idx -= 1;
        self.len -= 1;
        self.input[self.len] = 0;
        true
    }

    fn delete_forward(&mut self) -> bool {
        if self.idx >= self.len {
            return false;
        }

        self.input.copy_within(self.idx + 1..self.len, self.idx);
        self.len -= 1;
        self.input[self.len] = 0;
        true
    }

    fn idx_left(&mut self) -> bool {
        if self.idx == 0 {
            return false;
        }
        self.idx -= 1;
        true
    }

    fn idx_right(&mut self) -> bool {
        if self.idx >= self.len {
            return false;
        }
        self.idx += 1;
        true
    }
    fn add_to_history(&mut self, command: &str) {
        if command.is_empty() {
            self.current_history_offset = 0;
            return;
        }

        let mut should_add = true;
        if self.history_idx > 0 {
            let last_idx = (self.history_idx - 1) % self.history.len();
            let last_cmd_len = self.history[last_idx]
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(MAX_INPUT_LEN);
            if let Ok(last_cmd_str) = str::from_utf8(&self.history[last_idx][..last_cmd_len]) {
                if last_cmd_str == command {
                    should_add = false;
                }
            }
        }

        if should_add {
            let idx = self.history_idx % self.history.len();
            self.history[idx].fill(0);
            let bytes = command.as_bytes();
            let len = bytes.len().min(MAX_INPUT_LEN);
            self.history[idx][..len].copy_from_slice(&bytes[..len]);
            self.history_idx += 1;
        }

        self.current_history_offset = 0;
    }

    fn history_up(&mut self) -> bool {
        let max_offset = self.history_idx.min(self.history.len());
        if self.history_idx == 0 || self.current_history_offset >= max_offset {
            return false;
        }

        if self.current_history_offset == 0 {
            self.saved_input.copy_from_slice(&self.input);
            self.saved_len = self.len;
        }

        let idx = (self.history_idx - 1 - self.current_history_offset) % self.history.len();
        self.current_history_offset += 1;
        self.load_from_buffer(self.history[idx]);
        true
    }

    fn history_down(&mut self) -> bool {
        if self.current_history_offset == 0 {
            return false;
        }

        if self.current_history_offset == 1 {
            // Restore the saved un-executed line
            self.current_history_offset = 0;
            self.input.copy_from_slice(&self.saved_input);
            self.len = self.saved_len;
            self.idx = self.len;
            return true;
        }

        self.current_history_offset -= 1;
        let idx = (self.history_idx - self.current_history_offset) % self.history.len();
        self.load_from_buffer(self.history[idx]);
        true
    }

    fn load_from_buffer(&mut self, buf: [u8; MAX_INPUT_LEN]) {
        self.input.copy_from_slice(&buf);
        self.len = buf.iter().position(|&b| b == 0).unwrap_or(MAX_INPUT_LEN);
        self.idx = self.len;
    }
}

struct ShellStateCell(UnsafeCell<ShellState>);

unsafe impl Sync for ShellStateCell {}

static SHELL_STATE: ShellStateCell = ShellStateCell(UnsafeCell::new(ShellState::new()));

pub(crate) fn with_shell_state_mut<R>(f: impl FnOnce(&mut ShellState) -> R) -> R {
    let state = unsafe { &mut *SHELL_STATE.0.get() };
    f(state)
}

#[inline]
pub(crate) fn print(s: &str) {
    print_str_on(SCREEN_INDEX, s);
}

#[inline]
pub(crate) fn print_char(c: char) {
    print_char_on(SCREEN_INDEX, c);
}

#[inline]
pub(crate) fn print_fmt(args: &fmt::Arguments<'_>) {
    print_fmt_on(SCREEN_INDEX, args);
}

pub(crate) fn handle_sigint() {
    print_char('\n');
    with_shell_state_mut(|state| {
        state.clear_input();
        state.current_history_offset = 0;
    });
    redraw_input_line();
}

pub fn init_shell() {
    with_shell_state_mut(|state| {
        if state.initialized {
            return;
        }

        state.initialized = true;
        if security::has_accounts() {
            state.login_stage = 1;
            print("login: ");
            state.rendered_len = 7;
        } else {
            print(PROMPT);
            state.rendered_len = PROMPT.len();
        }
        set_cursor_movement_on(SCREEN_INDEX, CursorMovement::Horizontal);
    });

    set_screen_accepts_input(SCREEN_INDEX, true);
}

pub fn handle_shell_key_event(event: KeyEvent, modifiers: Modifiers) -> bool {
    if !is_screen_active(SCREEN_INDEX) || !active_screen_accepts_input() {
        return false;
    }

    match event.key {
        KeyCode::Home => {
            with_shell_state_mut(|state| {
                state.idx = 0;
            });
            redraw_input_line();
            true
        }
        KeyCode::End => {
            with_shell_state_mut(|state| {
                state.idx = state.len;
            });
            redraw_input_line();
            true
        }
        KeyCode::ArrowLeft => {
            if modifiers.shift() {
                switch_to_previous_screen();
            } else if with_shell_state_mut(|state| state.idx_left()) {
                redraw_input_line();
            }
            true
        }
        KeyCode::ArrowRight => {
            if modifiers.shift() {
                switch_to_next_screen();
            } else if with_shell_state_mut(|state| state.idx_right()) {
                redraw_input_line();
            }
            true
        }
        KeyCode::ArrowUp => {
            let changed = with_shell_state_mut(|state| {
                if state.login_stage == 0 {
                    state.history_up()
                } else {
                    false
                }
            });
            if changed {
                redraw_input_line();
            }
            true
        }
        KeyCode::ArrowDown => {
            let changed = with_shell_state_mut(|state| {
                if state.login_stage == 0 {
                    state.history_down()
                } else {
                    false
                }
            });
            if changed {
                redraw_input_line();
            }
            true
        }
        KeyCode::Enter => {
            print_char('\n');
            run_command_line();
            true
        }
        KeyCode::Backspace => {
            let removed = with_shell_state_mut(|state| state.delete_char());
            if removed {
                redraw_input_line();
            }
            true
        }
        KeyCode::Delete => {
            let removed = with_shell_state_mut(|state| state.delete_forward());
            if removed {
                redraw_input_line();
            }
            true
        }
        KeyCode::Tab => {
            if with_shell_state_mut(|state| state.login_stage != 0) {
                return true;
            }
            with_shell_state_mut(|state| {
                if state.idx == 0 || state.input[state.idx - 1] == b' ' {
                    print("\n");
                    for cmd in COMMANDS {
                        print_fmt(&format_args!("{} ", cmd));
                    }
                    print("\n");
                    state.clear_input();
                    redraw_input_line();
                    return;
                }
                let start = state.input[..state.idx]
                    .iter()
                    .rposition(|&b| b == b' ')
                    .map_or(0, |pos| pos + 1);
                let partial = str::from_utf8(&state.input[start..state.idx]).unwrap_or("");
                let (common_prefix, match_count) = longest_common_prefix(partial);
                if match_count == 0 {
                    return;
                }

                let missing_prefix = &common_prefix[partial.len()..];
                if !missing_prefix.is_empty() {
                    for c in missing_prefix.chars() {
                        state.push_char(c);
                    }
                } else if match_count > 1 {
                    print("\n");
                    for cmd in COMMANDS.iter().filter(|&&cmd| cmd.starts_with(partial)) {
                        print_fmt(&format_args!("{} ", cmd));
                    }
                    print("\n");
                }

                redraw_input_line();
            });
            true
        }
        _ => {
            if !modifiers.has_text_blocking_modifier() {
                if let Some(ch) = keyboard::keycode_to_char(event.key, modifiers) {
                    let _ = try_insert_char(ch);
                    return true;
                }
            }
            false
        }
    }
}

fn longest_common_prefix(partial: &str) -> (&'static str, usize) {
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

fn try_insert_char(c: char) -> bool {
    let inserted = with_shell_state_mut(|state| state.push_char(c));
    if inserted {
        redraw_input_line();
    }
    inserted
}

fn redraw_input_line() {
    let (cursor_x, cursor_y) = active_cursor_position();

    with_shell_state_mut(|state| {
        let old_rendered_len = state.rendered_len;
        let prompt = if state.login_stage == 1 {
            "login: "
        } else if state.login_stage == 2 {
            "password: "
        } else if state.login_stage == 3 {
            "New password: "
        } else {
            PROMPT
        };
        let visible_len = if state.login_stage == 2 || state.login_stage == 3 {
            0
        } else {
            state.len
        };
        let new_rendered_len = prompt.len() + visible_len;

        print_char('\r');
        print(prompt);

        if state.login_stage != 2 && state.login_stage != 3 {
            if let Ok(input_str) = str::from_utf8(&state.input[..state.len]) {
                print(input_str);
            }
        }

        if old_rendered_len > new_rendered_len {
            for _ in 0..(old_rendered_len - new_rendered_len) {
                print_char(' ');
            }
        }

        let visible_idx = if state.login_stage == 2 || state.login_stage == 3 {
            0
        } else {
            state.idx
        };
        let cursor_offset = prompt.len() + visible_idx;
        let new_cursor_x = (cursor_offset % VGA_WIDTH) as u16;
        let new_cursor_y = cursor_y + (cursor_offset / VGA_WIDTH) as u16;
        set_cursor_position_on(SCREEN_INDEX, new_cursor_x, new_cursor_y);
        state.rendered_len = new_rendered_len;
        let _ = cursor_x;
        scroll_view_to_bottom();
    });
}

fn run_command_line() {
    let (len, mut line_buf) = with_shell_state_mut(|state| {
        let len = state.len;
        let mut buf = [0u8; MAX_INPUT_LEN];
        buf[..len].copy_from_slice(&state.input[..len]);
        state.clear_input();
        (len, buf)
    });

    let line = str::from_utf8(&line_buf[..len]).unwrap_or("");
    let line = line.trim();

    if with_shell_state_mut(|state| state.login_stage != 0) {
        commands::auth::handle_login_line(line, &line_buf[..len]);
        line_buf.fill(0);
        return;
    }

    if line.is_empty() {
        print(PROMPT);
        with_shell_state_mut(|state| {
            state.idx = 0;
            state.rendered_len = PROMPT.len();
        });
        return;
    }

    unsafe { run_command(line) };
    with_shell_state_mut(|state| {
        state.add_to_history(line);
    });

    if is_screen_active(SCREEN_INDEX) && with_shell_state_mut(|state| state.login_stage == 0) {
        print(PROMPT);
        with_shell_state_mut(|state| {
            state.idx = 0;
            state.rendered_len = PROMPT.len();
        });
    }
}

unsafe fn run_command(line: &str) {
    let mut parts: str::SplitWhitespace<'_> = line.split_whitespace();
    let Some(command) = parts.next() else {
        return;
    };

    match command {
        "help" => command_help(parts),
        "clear" => clear(SCREEN_INDEX),
        "echo" => {
            let rest = line[command.len()..].trim_start();
            print(rest);
            print_char('\n');
        }
        "reboot" => request_reboot(),
        "shutdown" => request_shutdown(),
        "screen" => commands::system::command_screen(parts),
        "loglevel" => commands::system::command_loglevel(parts),
        "color" => commands::system::command_color(parts),
        "memstat" => dump::print_memstat(|args| print_fmt(args)),
        "memdebug" => dump::print_memdebug(|args| print_fmt(args)),
        "memdump" => commands::memory::command_memdump(parts),
        "pte" => commands::memory::command_pte(parts),
        "memtest" => dump::run_memtest(line[command.len()..].trim(), |args| print_fmt(args)),
        "stack" => commands::memory::command_stack(parts),
        "layout" => commands::system::command_layout(parts),
        "crash" => {
            core::ptr::read_volatile(0xdeadbeef as *const u32);
        }
        "signal" => commands::process::command_signal(parts),
        "spawn" => commands::process::command_spawn(),
        "wait" => commands::process::command_wait(),
        "kill" => commands::process::command_kill(parts),
        "dump_vfs" => {
            fs::print_vfs_tree(fs::ROOT_NODE, 0);
        }
        "fs_test" => commands::fs::command_fs_test(),
        "ps" => commands::process::command_ps(),
        "fs_tree" => commands::fs::command_fs_tree(),
        "devices" => commands::fs::command_devices(),
        "storage_test" => commands::fs::command_storage_test(parts),
        "mkdir" => commands::fs::command_mkdir(parts),
        "mount" => commands::fs::command_mount(parts),
        "umount" => commands::fs::command_umount(parts),
        "useradd" => commands::auth::command_useradd(parts),
        "users" | "userlist" => commands::auth::command_users(),
        "login" => commands::auth::command_login(parts),
        "su" => commands::auth::command_su(parts),
        "passwd" => commands::auth::command_passwd(parts),
        "whoami" => commands::auth::command_whoami(),
        "logout" => commands::auth::command_logout(),
        "initcalls" => command_initcalls(),
        _ => {
            print("unknown command: ");
            print(command);
            print("\n");
        }
    }
}

fn command_initcalls() {
    print("=== Linux-Style Initcalls ===\n");
    if let Some((start, end, pages)) = crate::initcall::freed_memory_info() {
        print_fmt(&format_args!(
            "Init memory reclaimed: {:#010X} - {:#010X} ({} KiB, {} pages)\n",
            start,
            end,
            (end - start) / 1024,
            pages
        ));
    } else {
        print("Init memory: active / not freed\n");
    }

    let history = crate::initcall::boot_history();
    print_fmt(&format_args!("Boot initcalls executed: {}\n", history.len()));
    print("Lvl       Name                                Status  Address\n");
    print("-------   ----------------------------------  ------  ----------\n");
    for record in history {
        let lvl_name = crate::initcall::InitcallLevel::from_u8(record.level)
            .map_or("???", |l| l.as_str());
        let status = if record.result == 0 { "OK" } else { "ERR" };
        print_fmt(&format_args!(
            "[{:<7}] {:<34}  {:<6}  {:#010X}\n",
            lvl_name,
            record.name,
            status,
            record.func_addr
        ));
    }
}

#[inline(always)]
fn command_help(mut parts: str::SplitWhitespace<'_>) {
    let Some(topic) = parts.next() else {
        print("Available commands:\n");
        for cmd in COMMANDS {
            print_fmt(&format_args!("{:<10}", cmd));
        }
        print(
            "\n\nType 'help <command>' for more details on a specific command.\n\n\
            Tab for auto-completion.\n\
            Up/Down arrows for command history.\n\
            Home/End to move the cursor to the beginning or end of the line.\n\
            Shift+Left/Right arrows to switch between screens.\n\
            Ctrl+C to interrupt the current command.\n",
        );
        return;
    };

    print(match topic {
        "help" => "help [command]\n  Show this help message or details about a specific command.\n",
        "clear" => "clear\n  Clear the shell screen.\n",
        "echo" => "echo <message>\n  Print the message to the shell.\n",
        "reboot" => "reboot\n  Reboot the system.\n",
        "shutdown" => "shutdown\n  Shut down the system.\n",
        "screen" => "screen <1-6>\n  Switch to a different screen (virtual terminal).\n",
        "loglevel" => "loglevel <emerg|alert|crit|err|warn|notice|info|debug>\n  Set the kernel log level.\n",
        "color" => "color <white|gray|red|green|blue|yellow|cyan|magenta>\n  Change the shell text color.\n",
        "memstat" => "memstat\n  Display memory usage statistics.\n",
        "memdebug" => "memdebug\n  Display detailed memory allocator debug information.\n",
        "memdump" => "memdump <addr> [len<=512]\n  Dump virtual memory starting at addr for len bytes (default 128).\n",
        "pte" => "pte <addr>\n  Display the page table entry for the given virtual address.\n",
        "memtest" => "memtest [physical,vmem,heap,page,all]\n  Run memory tests on different memory regions.\n",
        "stack" => "stack [words<=64]\n  Dump the current stack contents (default 32 words).\n",
        "crash" => "crash\n  Intentionally crash the kernel for testing purposes.\n",
        "layout" => "layout <us|tr>\n  Change keyboard layout to US QWERTY or Turkish QWERTY.\n",
        "read_test" => "read_test\n  Test blocking read by prompting for user input and echoing it back.\n",
        "signal" => "signal <signum|signal_name>\n  Send a signal to the shell process\n",
        "wait" => "wait\n  Wait for a child process to exit and reap it if it's a zombie.\n",
        "sleep" => "sleep <milliseconds>\n  Put the shell process to sleep for the specified duration.\n",
        "kill" => "kill <pid>\n  Send SIGKILL to the specified process ID.\n",
        "ps" => "ps\n  Display information about running processes.\n",
        "fs_test" => "fs_test\n  Exercise mknod, mount, umount, open(O_CREAT|O_TRUNC), write, and read.\n",
        "spawn" => "spawn\n  Spawn a user process with 'test::most_syscalls_we_have_probably' entry point. Change the code for something else.\n",
        "fs_tree" => "fs_tree\n  Print the virtual file system tree starting from the root node.\n",
        "devices" => "devices\n  List registered disks and partitions.\n",
        "storage_test" => "storage_test <device-id>\n  Read and validate a device MBR or EXT2 superblock.\n",
        "mkdir" => "mkdir <path>\n  Create an empty directory.\n",
        "mount" => "mount <device-id> <target>\n  Mount an EXT2 partition at an empty directory.\n",
        "umount" => "umount <target>\n  Unmount a filesystem from a directory.\n",
        "useradd" => "useradd <name>\n  Add or update a user account.\n",
        "users" => "users\n  List existing user accounts (UID, GID, username).\n",
        "userlist" => "userlist\n  Alias for 'users'. List existing user accounts.\n",
        "login" => "login [user]\n  Log in with existing user credentials.\n",
        "su" => "su [user]\n  Switch user to root (default) or specified user.\n",
        "passwd" => "passwd [user]\n  Change user password.\n",
        "whoami" => "whoami\n  Print current user name.\n",
        "logout" => "logout\n  Log out from the current session.\n",
        "initcalls" => "initcalls\n  Display boot initcall execution history and reclaimed memory stats.\n",
        _ => "Unknown command. Type 'help' for a list of commands.\n",
    });
}
