use core::cell::UnsafeCell;
use core::fmt;
use core::str;

use crate::debug::{
    memory,
    stack::{self, DumpStackOptions},
};
use crate::interrupts::keyboard::character_map::keycode_to_char;
use crate::interrupts::keyboard::keycode::{KeyCode, KeyEvent, Modifiers};
use crate::interrupts::utils::{request_reboot, request_shutdown};
use crate::printk::{set_log_level, KernelLogLevel};
use crate::signals::Signal;
use crate::startup_config;
use crate::vga::text_mod::out::{
    self, active_screen_accepts_input, change_color, clear, print_char_on, print_on,
    set_cursor_movement_on, switch_screen, Color, ColorCode,
};
use crate::vga::text_mod::screen;

const PROMPT: &str = "mysh > ";
const MAX_INPUT_LEN: usize = startup_config::shell::MAX_INPUT_LEN;
const SCREEN_INDEX: usize = startup_config::shell::SCREEN_INDEX;

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
    print_on(SCREEN_INDEX, s);
}

#[inline]
fn print_char(c: char) {
    print_char_on(SCREEN_INDEX, c);
}

#[inline]
fn print_fmt(args: &fmt::Arguments<'_>) {
    out::write_fmt_on(SCREEN_INDEX, args);
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
        print(
            "This is the default screen for the shell \n\
            Use F1-F6 / Shift+<Left/Right Arrow> to switch screens.\n",
        );
        print(PROMPT);
        state.rendered_len = PROMPT.len();
        set_cursor_movement_on(SCREEN_INDEX, screen::CursorMovement::Horizontal);
    });

    crate::signals::register_signal_handler(Signal::SIGINT, shell_sigint_handler);
}

pub fn handle_shell_key_event(event: KeyEvent, modifiers: Modifiers) -> bool {
    if !out::is_screen_active(SCREEN_INDEX) || !active_screen_accepts_input() {
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
                out::switch_to_previous_screen();
            } else if with_shell_state_mut(|state| state.idx_left()) {
                redraw_input_line();
            }
            true
        }
        KeyCode::ArrowRight => {
            if modifiers.shift() {
                out::switch_to_next_screen();
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
                    print(
                        "\nhelp clear echo shutdown reboot screen loglevel color memstat memdebug \n
                        memdump pte memtest stack\n",
                    );
                    state.clear_input();
                    redraw_input_line();
                    return;
                }
                let start = state.input[..state.idx]
                    .iter()
                    .rposition(|&b| b == b' ')
                    .map_or(0, |pos| pos + 1);
                let partial = str::from_utf8(&state.input[start..state.idx]).unwrap_or("");
                if let Some(completion) = complete(partial) {
                    for c in completion[partial.len()..].chars() {
                        state.push_char(c);
                    }
                    redraw_input_line();
                }
            });
            true
        }
        _ => {
            if !modifiers.has_text_blocking_modifier() {
                if let Some(ch) = keycode_to_char(event.key, modifiers) {
                    let _ = try_insert_char(ch);
                    return true;
                }
            }
            false
        }
    }
}

fn try_insert_char(c: char) -> bool {
    let inserted = with_shell_state_mut(|state| state.push_char(c));
    if inserted {
        redraw_input_line();
    }
    inserted
}

fn redraw_input_line() {
    let (cursor_x, cursor_y) = out::active_cursor_position();

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
        let new_cursor_x = (cursor_offset % screen::VGA_WIDTH) as u16;
        let new_cursor_y = cursor_y + (cursor_offset / screen::VGA_WIDTH) as u16;
        out::set_cursor_position_on(SCREEN_INDEX, new_cursor_x, new_cursor_y);
        state.rendered_len = new_rendered_len;
        let _ = cursor_x;
        out::scroll_view_to_bottom();
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

    run_command(line);
    with_shell_state_mut(|state| {
        state.add_to_history(line);
    });

    if out::is_screen_active(SCREEN_INDEX) {
        print(PROMPT);
        with_shell_state_mut(|state| {
            state.idx = 0;
            state.rendered_len = PROMPT.len();
        });
    }
}

fn run_command(line: &str) {
    let mut parts: str::SplitWhitespace<'_> = line.split_whitespace();
    let Some(command) = parts.next() else {
        return;
    };

    match command {
        "help" => command_help(),
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
        "memstat" => memory::print_memstat(|args| print_fmt(args)),
        "memdebug" => memory::print_memdebug(|args| print_fmt(args)),
        "memdump" => command_memdump(parts),
        "pte" => command_pte(parts),
        "memtest" => {
            let features = line[command.len()..].trim();
            memory::run_memtest(features, |args| print_fmt(args));
        }
        "stack" => {
            let words = if let Some(words_str) = parts.next() {
                let Some(parsed) = parse_usize(words_str) else {
                    print("invalid word count\n");
                    return;
                };
                parsed
            } else {
                stack::DEFAULT_DUMP_WORDS
            };

            if words == 0 || words > stack::MAX_DUMP_WORDS {
                print("word count must be in range 1..=64\n");
                return;
            }

            command_stack(words);
        }
        "layout" => command_layout(parts),
        "crash" => {
            unsafe {
                // SAFETY: This is intentionally crashing the kernel for testing purposes.
                core::ptr::read_volatile(0xdeadbeef as *const u32);
            }
        }
        "sc_write" => command_sc_write(),
        "read_test" => command_read_test(),
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

fn complete(_partial: &str) -> Option<&'static str> {
    let commands = [
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
        "sc_write",
        "layout",
        "read_test",
    ];
    let mut matches = commands.iter().filter(|&cmd| cmd.starts_with(_partial));
    let first_match = matches.next()?;
    if matches.next().is_some() {
        print_char('\n');
        print(*first_match);
        matches.for_each(|&cmd| print_fmt(&format_args!(" {}", cmd)));
        print_char('\n');
        redraw_input_line();
        None
    } else {
        Some(first_match)
    }
}

fn parse_u32(input: &str) -> Option<u32> {
    if let Some(hex) = input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16).ok()
    } else {
        input.parse::<u32>().ok()
    }
}

fn parse_usize(input: &str) -> Option<usize> {
    if let Some(hex) = input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
    {
        usize::from_str_radix(hex, 16).ok()
    } else {
        input.parse::<usize>().ok()
    }
}

#[inline(always)]
fn command_help() {
    print(
        r#"Commands:
        help clear echo shutdown reboot screen loglevel color memstat memdebug
        memdump pte memtest stack crash sc_write layout read_test
          screen <1-6>
          loglevel <emerg|alert|crit|err|warn|notice|info|debug>
          color <white|gray|red|green|blue|yellow|cyan|magenta>
          memstat
          memdebug
          memdump <addr> [len<=512]
          pte <addr>
          memtest [physical,vmem,heap,page,all]
          stack [words<=64]
          crash
          sc_write
          layout <us|tr>
          read_test
        "#,
    );
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
        memory::MEMDUMP_DEFAULT_LEN
    };

    if len == 0 || len > memory::MEMDUMP_MAX_LEN {
        print("length must be in range 1..=512\n");
        return;
    }

    memory::dump_virtual_memory(addr, len, |args| print_fmt(args));
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

    memory::debug_page_entry(addr, |args| print_fmt(args));
}

#[inline(always)]
fn command_stack(words: usize) {
    let options = DumpStackOptions { words, trace_frames: stack::DEFAULT_TRACE_FRAMES };

    stack::dump_stack_with_options(options, |args| {
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
        "us" => crate::interrupts::keyboard::character_map::KeyboardLayout::UsQwerty,
        "tr" => crate::interrupts::keyboard::character_map::KeyboardLayout::TrQwerty,
        _ => {
            print("invalid layout\n");
            return;
        }
    };

    crate::interrupts::keyboard::character_map::set_layout(layout);
    print("keyboard layout updated\n");
}

#[inline(always)]
fn command_sc_write() {
    let msg = "This message was printed using the sc_write command.\n";
    let fd: i32 = 1; // stdout
    let buf_ptr = msg.as_ptr();
    let len = msg.len();
    unsafe {
        // 32-bit x86 syscall convention using int 0x80
        core::arch::asm!(
            "int 0x80",
            in("eax") 4,
            in("ebx") fd,
            in("ecx") buf_ptr,
            in("edx") len,
            options(nostack, nomem),
        );
    }
}

#[inline(always)]
fn command_read_test() {
    print("Entering blocking read mode.\n");
    print("Please type your name: ");

    let mut buffer = [0u8; 64];

    // This will block the shell execution until the user presses Enter
    let len = crate::interrupts::keyboard::api::get_line(&mut buffer);

    if let Ok(input_str) = core::str::from_utf8(&buffer[..len]) {
        print("Hello, ");
        print(input_str);
        print("!\n");
    } else {
        print("Invalid input.\n");
    }
}
