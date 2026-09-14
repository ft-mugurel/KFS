use core::cell::UnsafeCell;
use core::fmt;
use core::str;

use crate::{
    drivers,
    dump::{
        self, DumpStackOptions, DEFAULT_DUMP_WORDS, DEFAULT_TRACE_FRAMES, MAX_DUMP_WORDS,
        MEMDUMP_DEFAULT_LEN, MEMDUMP_MAX_LEN,
    },
    fs,
    interrupts::{
        keyboard::{self, KeyCode, KeyEvent, KeyboardLayout, Modifiers},
        request_reboot,
    },
    pr_info,
    printk::{set_log_level, KernelLogLevel},
    sched,
    signals::{self, Signal},
    smp::ipi::request_shutdown,
    startup_config, test,
    vga::text_mod::{
        active_cursor_position, active_screen_accepts_input, change_color, clear, is_screen_active,
        print_char_on, print_fmt_on, print_str_on, scroll_view_to_bottom, set_cursor_movement_on,
        set_cursor_position_on, set_screen_accepts_input, switch_screen, switch_to_next_screen,
        switch_to_previous_screen, Color, ColorCode, CursorMovement, VGA_WIDTH,
    },
};

const PROMPT: &str = "mysh > ";
const MAX_INPUT_LEN: usize = startup_config::shell::MAX_INPUT_LEN;
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
];

struct ShellState {
    input: [u8; MAX_INPUT_LEN],
    idx: usize,
    len: usize,
    rendered_len: usize,
    initialized: bool,
    history: [[u8; MAX_INPUT_LEN]; 16],
    history_idx: usize,
    current_history_offset: usize,
    saved_input: [u8; MAX_INPUT_LEN],
    saved_len: usize,
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
        }
    }

    fn clear_input(&mut self) {
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

fn with_shell_state_mut<R>(f: impl FnOnce(&mut ShellState) -> R) -> R {
    let state = unsafe { &mut *SHELL_STATE.0.get() };
    f(state)
}

#[inline]
fn print(s: &str) {
    print_str_on(SCREEN_INDEX, s);
}

#[inline]
fn print_char(c: char) {
    print_char_on(SCREEN_INDEX, c);
}

#[inline]
fn print_fmt(args: &fmt::Arguments<'_>) {
    print_fmt_on(SCREEN_INDEX, args);
}

fn shell_sigint_handler() {
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
        print(PROMPT);
        state.rendered_len = PROMPT.len();
        set_cursor_movement_on(SCREEN_INDEX, CursorMovement::Horizontal);
    });

    unsafe { signals::register_signal_handler(Signal::SIGINT, shell_sigint_handler as u32) };
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
            let changed = with_shell_state_mut(|state| state.history_up());
            if changed {
                redraw_input_line();
            }
            true
        }
        KeyCode::ArrowDown => {
            let changed = with_shell_state_mut(|state| state.history_down());
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
        let new_rendered_len = PROMPT.len() + state.len;

        print_char('\r');
        print(PROMPT);

        if let Ok(input_str) = str::from_utf8(&state.input[..state.len]) {
            print(input_str);
        }

        if old_rendered_len > new_rendered_len {
            for _ in 0..(old_rendered_len - new_rendered_len) {
                print_char(' ');
            }
        }

        let cursor_offset = PROMPT.len() + state.idx;
        let new_cursor_x = (cursor_offset % VGA_WIDTH) as u16;
        let new_cursor_y = cursor_y + (cursor_offset / VGA_WIDTH) as u16;
        set_cursor_position_on(SCREEN_INDEX, new_cursor_x, new_cursor_y);
        state.rendered_len = new_rendered_len;
        let _ = cursor_x;
        scroll_view_to_bottom();
    });
}

fn run_command_line() {
    let (len, line_buf) = with_shell_state_mut(|state| {
        let len = state.len;
        let mut buf = [0u8; MAX_INPUT_LEN];
        buf[..len].copy_from_slice(&state.input[..len]);
        state.clear_input();
        (len, buf)
    });

    let line = str::from_utf8(&line_buf[..len]).unwrap_or("");
    let line = line.trim();

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

    if is_screen_active(SCREEN_INDEX) {
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
        "screen" => command_screen(parts),
        "loglevel" => command_loglevel(parts),
        "color" => command_color(parts),
        "memstat" => dump::print_memstat(|args| print_fmt(args)),
        "memdebug" => dump::print_memdebug(|args| print_fmt(args)),
        "memdump" => command_memdump(parts),
        "pte" => command_pte(parts),
        "memtest" => dump::run_memtest(line[command.len()..].trim(), |args| print_fmt(args)),
        "stack" => command_stack(parts),
        "layout" => command_layout(parts),
        "crash" => {
            core::ptr::read_volatile(0xdeadbeef as *const u32);
        }
        "signal" => command_signal(parts),
        "spawn" => command_spawn(),
        "wait" => command_wait(),
        "kill" => command_kill(parts),
        "dump_vfs" => unsafe {
            fs::print_vfs_tree(fs::ROOT_NODE, 0);
        },
        "fs_test" => command_fs_test(),
        "ps" => command_ps(),
        "fs_tree" => command_fs_tree(),
        "devices" => command_devices(),
        "storage_test" => command_storage_test(parts),
        "mkdir" => command_mkdir(parts),
        "mount" => command_mount(parts),
        "umount" => command_umount(parts),
        _ => {
            print("unknown command: ");
            print(command);
            print("\n");
        }
    }
}

fn parse_log_level(name: &str) -> Option<KernelLogLevel> {
    match name {
        "emerg" => Some(KernelLogLevel::Emerg),
        "alert" => Some(KernelLogLevel::Alert),
        "crit" => Some(KernelLogLevel::Crit),
        "err" => Some(KernelLogLevel::Err),
        "warn" | "warning" => Some(KernelLogLevel::Warning),
        "notice" => Some(KernelLogLevel::Notice),
        "info" => Some(KernelLogLevel::Info),
        "debug" => Some(KernelLogLevel::Debug),
        _ => None,
    }
}

fn parse_color(name: &str) -> Option<Color> {
    match name {
        "white" => Some(Color::White),
        "gray" | "lightgray" => Some(Color::LightGray),
        "red" => Some(Color::LightRed),
        "green" => Some(Color::LightGreen),
        "blue" => Some(Color::LightBlue),
        "yellow" => Some(Color::Yellow),
        "cyan" => Some(Color::LightCyan),
        "magenta" => Some(Color::Pink),
        _ => None,
    }
}

fn strip_hex_prefix(input: &str) -> Option<&str> {
    let input = input.trim();
    input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
}

fn parse_u32(input: &str) -> Option<u32> {
    let input = input.trim();
    if let Some(hex) = strip_hex_prefix(input) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        input.parse::<u32>().ok()
    }
}

fn parse_usize(input: &str) -> Option<usize> {
    let input = input.trim();
    if let Some(hex) = strip_hex_prefix(input) {
        usize::from_str_radix(hex, 16).ok()
    } else {
        input.parse::<usize>().ok()
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

    match topic {
        "help" => print("help [command]\n  Show this help message or details about a specific command.\n"),
        "clear" => print("clear\n  Clear the shell screen.\n"),
        "echo" => print("echo <message>\n  Print the message to the shell.\n"),
        "reboot" => print("reboot\n  Reboot the system.\n"),
        "shutdown" => print("shutdown\n  Shut down the system.\n"),
        "screen" => print("screen <1-6>\n  Switch to a different screen (virtual terminal).\n"),
        "loglevel" => print("loglevel <emerg|alert|crit|err|warn|notice|info|debug>\n  Set the kernel log level.\n"),
        "color" => print("color <white|gray|red|green|blue|yellow|cyan|magenta>\n  Change the shell text color.\n"),
        "memstat" => print("memstat\n  Display memory usage statistics.\n"),
        "memdebug" => print("memdebug\n  Display detailed memory allocator debug information.\n"),
        "memdump" => print("memdump <addr> [len<=512]\n  Dump virtual memory starting at addr for len bytes (default 128).\n"),
        "pte" => print("pte <addr>\n  Display the page table entry for the given virtual address.\n"),
        "memtest" => print("memtest [physical,vmem,heap,page,all]\n  Run memory tests on different memory regions.\n"),
        "stack" => print("stack [words<=64]\n  Dump the current stack contents (default 32 words).\n"),
        "crash" => print("crash\n  Intentionally crash the kernel for testing purposes.\n"),
        "layout" => print("layout <us|tr>\n  Change keyboard layout to US QWERTY or Turkish QWERTY.\n"),
        "read_test" => print("read_test\n  Test blocking read by prompting for user input and echoing it back.\n"),
        "signal" => print("signal <signum|signal_name>\n  Send a signal to the shell process\n"),
        "wait" => print("wait\n  Wait for a child process to exit and reap it if it's a zombie.\n"),
        "sleep" => print("sleep <milliseconds>\n  Put the shell process to sleep for the specified duration.\n"),
        "kill" => print("kill <pid>\n  Send SIGKILL to the specified process ID.\n"),
        "ps" => print("ps\n  Display information about running processes.\n"),
        "fs_test" => print("fs_test\n  Exercise mknod, mount, umount, open(O_CREAT|O_TRUNC), write, and read.\n"),
        "spawn" => print("spawn\n  Spawn a user process with 'test::most_syscalls_we_have_probably' entry point. Change the code for something else.\n"),
        "fs_tree" => print("fs_tree\n  Print the virtual file system tree starting from the root node.\n"),
        "devices" => print("devices\n  List registered disks and partitions.\n"),
        "storage_test" => print("storage_test <device-id>\n  Read and validate a device MBR or EXT2 superblock.\n"),
        "mkdir" => print("mkdir <path>\n  Create an empty directory.\n"),
        "mount" => print("mount <device-id> <target>\n  Mount an EXT2 partition at an empty directory.\n"),
        "umount" => print("umount <target>\n  Unmount a filesystem from a directory.\n"),
        _ => print("Unknown command. Type 'help' for a list of commands.\n"),
    }
}

#[inline(always)]
fn command_screen(mut parts: str::SplitWhitespace<'_>) {
    let Some(arg) = parts.next() else {
        print("usage: screen <1-6>\n");
        return;
    };

    let Ok(screen) = arg.parse::<usize>() else {
        print("invalid screen index\n");
        return;
    };

    if !(1..=6).contains(&screen) {
        print("screen index must be in range 1-6\n");
        return;
    }

    switch_screen(screen - 1);
}

#[inline(always)]
fn command_loglevel(mut parts: str::SplitWhitespace<'_>) {
    let Some(arg) = parts.next() else {
        print("usage: loglevel <emerg|alert|crit|err|warn|notice|info|debug>\n");
        return;
    };

    let Some(level) = parse_log_level(arg) else {
        print("invalid log level\n");
        return;
    };

    set_log_level(level);
    print("log level updated\n");
}

#[inline(always)]
fn command_color(mut parts: str::SplitWhitespace<'_>) {
    let Some(arg) = parts.next() else {
        print("usage: color <white|gray|red|green|blue|yellow|cyan|magenta>\n");
        return;
    };

    let Some(color) = parse_color(arg) else {
        print("invalid color\n");
        return;
    };

    change_color(ColorCode::new(color, Color::Black));
    print("shell color updated\n");
}

#[inline(always)]
fn command_memdump(mut parts: str::SplitWhitespace<'_>) {
    let Some(addr_str) = parts.next() else {
        print("usage: memdump <addr> [len<=512]\n");
        return;
    };

    let Some(addr) = parse_u32(addr_str) else {
        print("invalid address\n");
        return;
    };

    let len = if let Some(len_str) = parts.next() {
        let Some(parsed) = parse_usize(len_str) else {
            print("invalid length\n");
            return;
        };
        parsed
    } else {
        MEMDUMP_DEFAULT_LEN
    };

    if len == 0 || len > MEMDUMP_MAX_LEN {
        print("length must be in range 1..=512\n");
        return;
    }

    dump::dump_virtual_memory(addr, len, |args| print_fmt(args));
}

#[inline(always)]
fn command_pte(mut parts: str::SplitWhitespace<'_>) {
    let Some(addr_str) = parts.next() else {
        print("usage: pte <addr>\n");
        return;
    };

    let Some(addr) = parse_u32(addr_str) else {
        print("invalid address\n");
        return;
    };

    dump::debug_page_entry(addr, |args| print_fmt(args));
}

#[inline(always)]
fn command_stack(mut parts: str::SplitWhitespace<'_>) {
    let words = if let Some(words_str) = parts.next() {
        let Some(parsed) = parse_usize(words_str) else {
            print("invalid word count\n");
            return;
        };
        parsed
    } else {
        DEFAULT_DUMP_WORDS
    };

    if words == 0 || words > MAX_DUMP_WORDS {
        print("word count must be in range 1..=64\n");
        return;
    }

    let options = DumpStackOptions {
        words,
        frames: DEFAULT_TRACE_FRAMES,
        print_stack_values: true,
        walk_frames: true,
        scan_stack: true,
    };

    dump::dump_stack_with_options(options, |args| {
        print_fmt(&args);
    });
}

#[inline(always)]
fn command_layout(mut parts: str::SplitWhitespace<'_>) {
    let Some(layout_str) = parts.next() else {
        print("usage: layout <us|tr>\n");
        return;
    };

    let layout = match layout_str {
        "us" => KeyboardLayout::UsQwerty,
        "tr" => KeyboardLayout::TrQwerty,
        _ => {
            print("invalid layout\n");
            return;
        }
    };

    keyboard::set_layout(layout);
    print("keyboard layout updated\n");
}

#[inline(always)]
fn command_devices() {
    print("ID  NAME       KIND       PARENT  START     SECTORS\n");
    for device_id in 0..drivers::MAX_BLOCK_DEVICES {
        let Some(device) = drivers::device_at(device_id) else {
            continue;
        };
        let name_len = device
            .name
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(device.name.len());
        let name = core::str::from_utf8(&device.name[..name_len]).unwrap_or("?");
        let kind = if device.partition {
            "partition"
        } else {
            "disk"
        };
        print_fmt(&format_args!(
            "{:<3} {:<10} {:<10} {:<7} {:<9} {}\n",
            device_id, name, kind, device.parent, device.start_lba, device.sector_count
        ));
    }
}

#[inline(always)]
fn command_storage_test(mut parts: str::SplitWhitespace<'_>) {
    let Some(device_str) = parts.next() else {
        print("usage: storage_test <device-id>\n");
        return;
    };
    let Some(device_id) = parse_usize(device_str) else {
        print("invalid device id\n");
        return;
    };
    let Some(device) = drivers::device_at(device_id) else {
        print("device not found\n");
        return;
    };

    let mut buffer = [0u8; 1024];
    if drivers::read_sectors(device_id, 0, 1, &mut buffer).is_err() {
        print("device read failed\n");
        return;
    }

    if device.partition {
        if drivers::read_sectors(device_id, 2, 2, &mut buffer).is_err() {
            print("partition superblock read failed\n");
            return;
        }
        let magic = u16::from_le_bytes([buffer[56], buffer[57]]);
        print_fmt(&format_args!(
            "device {} EXT2 magic: {:#06x}\n",
            device_id, magic
        ));
        if magic == 0xEF53 {
            print("EXT2 superblock valid\n");
        } else {
            print("invalid EXT2 superblock\n");
        }
    } else if buffer[510] == 0x55 && buffer[511] == 0xAA {
        print("valid MBR signature\n");
        for index in 0..4 {
            let offset = 446 + index * 16;
            let partition_type = buffer[offset + 4];
            let start = u32::from_le_bytes([
                buffer[offset + 8],
                buffer[offset + 9],
                buffer[offset + 10],
                buffer[offset + 11],
            ]);
            let sectors = u32::from_le_bytes([
                buffer[offset + 12],
                buffer[offset + 13],
                buffer[offset + 14],
                buffer[offset + 15],
            ]);
            if partition_type != 0 && sectors != 0 {
                print_fmt(&format_args!(
                    "partition {}: type {:#04x}, start {}, sectors {}\n",
                    index + 1,
                    partition_type,
                    start,
                    sectors
                ));
            }
        }
    } else {
        print("no valid MBR signature\n");
    }
}

#[inline(always)]
unsafe fn command_mount(mut parts: str::SplitWhitespace<'_>) {
    let Some(device_str) = parts.next() else {
        print("usage: mount <device-id> <target>\n");
        return;
    };
    let Some(device_id) = parse_usize(device_str) else {
        print("invalid device id\n");
        return;
    };
    let Some(target_path) = parts.next() else {
        print("usage: mount <device-id> <target>\n");
        return;
    };
    let cwd = (*sched::current().as_mut().unwrap()).cwd;
    let target = match fs::resolve_path(target_path, cwd) {
        Ok(node) => node,
        Err(error) => {
            print_fmt(&format_args!("mount target lookup failed: {:?}\n", error));
            return;
        }
    };
    match fs::ext2::mount_device_at(device_id, target) {
        Ok(()) => print("filesystem mounted\n"),
        Err(error) => print_fmt(&format_args!("mount failed: {:?}\n", error)),
    }
}

#[inline(always)]
unsafe fn command_mkdir(mut parts: str::SplitWhitespace<'_>) {
    let Some(path) = parts.next() else {
        print("usage: mkdir <path>\n");
        return;
    };
    if path.is_empty() || path == "/" {
        print("invalid directory path\n");
        return;
    }

    let (parent_path, name) = match path.rsplit_once('/') {
        Some((parent, name)) => (if parent.is_empty() { "/" } else { parent }, name),
        None => ("", path),
    };
    if name.is_empty() {
        print("invalid directory path\n");
        return;
    }

    let cwd = (*sched::current().as_mut().unwrap()).cwd;
    let parent = match fs::resolve_path(parent_path, cwd) {
        Ok(node) => node,
        Err(error) => {
            print_fmt(&format_args!("mkdir parent lookup failed: {:?}\n", error));
            return;
        }
    };
    if (*parent).node_type != fs::VfsNodeType::Directory {
        print("mkdir parent is not a directory\n");
        return;
    }

    match fs::create_child_node(parent, name, fs::VfsNodeType::Directory, 0o755) {
        Ok(_) => print("directory created\n"),
        Err(error) => print_fmt(&format_args!("mkdir failed: {:?}\n", error)),
    }
}

#[inline(always)]
unsafe fn command_umount(mut parts: str::SplitWhitespace<'_>) {
    let Some(target_path) = parts.next() else {
        print("usage: umount <target>\n");
        return;
    };
    let cwd = (*sched::current().as_mut().unwrap()).cwd;
    let target = match fs::resolve_path(target_path, cwd) {
        Ok(node) => node,
        Err(error) => {
            print_fmt(&format_args!("unmount target lookup failed: {:?}\n", error));
            return;
        }
    };
    if target == fs::ROOT_NODE {
        print("cannot unmount the root filesystem\n");
        return;
    }
    (*target).children = core::ptr::null_mut();
    match fs::umount_node(target) {
        Ok(()) => print("filesystem unmounted\n"),
        Err(error) => print_fmt(&format_args!("unmount failed: {:?}\n", error)),
    }
}

#[inline(always)]
fn command_fs_test() {
    unsafe {
        let dev_path: [u8; 5] = [b'/', b'd', b'e', b'v', 0];
        let mnt_path: [u8; 5] = [b'/', b'm', b'n', b't', 0];
        let file_path: [u8; 13] = [
            b'/', b'f', b's', b'_', b't', b'e', b's', b't', b'.', b't', b'x', b't', 0,
        ];
        let write_msg: [u8; 6] = [b'f', b's', b'-', b'o', b'k', b'\n'];
        let mut result: u32;
        let mut fd: u32;
        let mut read_back: u32;
        let mut read_buf = [0u8; 32];

        core::arch::asm!(
            "int 0x80",
            in("eax") 8,
            in("ebx") mnt_path.as_ptr(),
            in("ecx") 0x4000u32 | 0o755,
            lateout("eax") _,
            options(nostack, nomem),
        );

        core::arch::asm!(
            "int 0x80",
            in("eax") 9,
            in("ebx") dev_path.as_ptr(),
            in("ecx") mnt_path.as_ptr(),
            lateout("eax") result,
            options(nostack, nomem),
        );
        if (result as i32) < 0 {
            print("fs_test: mount failed\n");
            return;
        }

        core::arch::asm!(
            "int 0x80",
            in("eax") 10,
            in("ebx") mnt_path.as_ptr(),
            lateout("eax") result,
            options(nostack, nomem),
        );
        if (result as i32) < 0 {
            print("fs_test: umount failed\n");
            return;
        }

        core::arch::asm!(
            "int 0x80",
            in("eax") 5,
            in("ebx") file_path.as_ptr(),
            in("ecx") 0x241u32, // O_CREAT | O_TRUNC | O_RDWR
            in("edx") 0o644u32,
            lateout("eax") fd,
            options(nostack, nomem),
        );
        if (fd as i32) < 0 {
            print("fs_test: open create/trunc failed\n");
            return;
        }

        core::arch::asm!(
            "int 0x80",
            in("eax") 4,
            in("ebx") fd,
            in("ecx") write_msg.as_ptr(),
            in("edx") write_msg.len(),
            lateout("eax") _,
            options(nostack, nomem),
        );
        core::arch::asm!("int 0x80", in("eax") 6, in("ebx") fd, options(nostack, nomem));

        core::arch::asm!(
            "int 0x80",
            in("eax") 5,
            in("ebx") file_path.as_ptr(),
            in("ecx") 0u32,
            in("edx") 0u32,
            lateout("eax") fd,
            options(nostack, nomem),
        );
        if (fd as i32) < 0 {
            print("fs_test: reopen failed\n");
            return;
        }

        core::arch::asm!(
            "int 0x80",
            in("eax") 3,
            in("ebx") fd,
            in("ecx") read_buf.as_mut_ptr(),
            in("edx") read_buf.len(),
            lateout("eax") read_back,
            options(nostack, nomem),
        );
        core::arch::asm!("int 0x80", in("eax") 6, in("ebx") fd, options(nostack, nomem));

        if (read_back as i32) < 0 {
            print("fs_test: readback failed\n");
            return;
        }

        print("fs_test: success\n");
        if let Ok(text) = core::str::from_utf8(&read_buf[..read_back as usize]) {
            print(text);
            print("\n");
        }
    }
}

#[inline(always)]
fn command_wait() {
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
fn command_signal(mut parts: str::SplitWhitespace<'_>) {
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
fn command_kill(mut parts: str::SplitWhitespace<'_>) {
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
fn command_spawn() {
    // if unsafe { sched::create_user_process(test::most_syscalls_we_have_probably, 1024) } {
    if unsafe { sched::create_user_process(test::most_syscalls_we_have_probably, 1024) } {
        pr_info!("Spawned user process with 1024 bytes.\n");
    } else {
        pr_info!("Failed to spawn user process. PID might already be in use.\n");
    }
}

#[inline(always)]
fn command_ps() {
    print(" PID | UID  | Parent PID | Exit Code  | State\n");
    for process in sched::PROCESS_TABLE.lock().iter() {
        if let Some(task) = process {
            print_fmt(&format_args!(" {:<3} |", task.pid));
            print_fmt(&format_args!(" {:<4} |", task.uid));
            print_fmt(&format_args!(" {:<10} |", task.family.parent_pid));
            print_fmt(&format_args!(" {:<10} |", task.exit_code.unwrap_or(0)));
            print_fmt(&format_args!(" {:<20}\n", task.state));
        }
    }
}

#[inline(always)]
fn command_fs_tree() {
    unsafe {
        fs::print_vfs_tree(fs::ROOT_NODE, 0);
    }
}
