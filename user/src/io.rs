// =============================================================================
// KFS User-Space I/O, Key Parsing & Line Editor
// =============================================================================

use crate::shell::{History, COMMANDS, MAX_INPUT_LEN};
use crate::syscall::{sys_read, sys_write};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Delete,
    Tab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Unknown,
}

pub fn print_str(s: &str) {
    sys_write(1, s.as_bytes());
}

pub fn print_bytes(b: &[u8]) {
    sys_write(1, b);
}

pub fn print_dec(mut n: usize) {
    if n == 0 {
        print_str("0");
        return;
    }
    let mut buf = [0u8; 20];
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    let mut rev = [0u8; 20];
    for j in 0..i {
        rev[j] = buf[i - 1 - j];
    }
    sys_write(1, &rev[..i]);
}

pub fn parse_dec(s: &str) -> Option<usize> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut res = 0usize;
    for b in s.bytes() {
        if (b'0'..=b'9').contains(&b) {
            res = res.checked_mul(10)?.checked_add((b - b'0') as usize)?;
        } else {
            return None;
        }
    }
    Some(res)
}

pub fn parse_hex(s: &str) -> Option<u32> {
    let s = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    if s.is_empty() {
        return None;
    }
    let mut res = 0u32;
    for b in s.bytes() {
        let val = match b {
            b'0'..=b'9' => (b - b'0') as u32,
            b'a'..=b'f' => (b - b'a' + 10) as u32,
            b'A'..=b'F' => (b - b'A' + 10) as u32,
            _ => return None,
        };
        res = (res << 4) | val;
    }
    Some(res)
}

pub fn read_byte() -> u8 {
    let mut b = [0u8; 1];
    loop {
        let n = sys_read(0, &mut b);
        if n == 1 {
            return b[0];
        }
    }
}

pub fn read_key() -> Key {
    let b = read_byte();
    match b {
        b'\n' | b'\r' => Key::Enter,
        0x08 | 0x7F => Key::Backspace,
        b'\t' => Key::Tab,
        0x1B => {
            let b2 = read_byte();
            if b2 == b'[' {
                let b3 = read_byte();
                match b3 {
                    b'A' => Key::Up,
                    b'B' => Key::Down,
                    b'C' => Key::Right,
                    b'D' => Key::Left,
                    b'H' => Key::Home,
                    b'F' => Key::End,
                    b'1' => {
                        let b4 = read_byte();
                        if b4 == b'~' {
                            Key::Home
                        } else {
                            Key::Unknown
                        }
                    }
                    b'3' => {
                        let b4 = read_byte();
                        if b4 == b'~' {
                            Key::Delete
                        } else {
                            Key::Unknown
                        }
                    }
                    b'4' => {
                        let b4 = read_byte();
                        if b4 == b'~' {
                            Key::End
                        } else {
                            Key::Unknown
                        }
                    }
                    _ => Key::Unknown,
                }
            } else {
                Key::Unknown
            }
        }
        32..=126 => Key::Char(b as char),
        _ => Key::Unknown,
    }
}

pub fn redraw(prompt: &str, buf: &[u8], len: usize, idx: usize, old_len: usize) {
    print_str("\r");
    print_str(prompt);
    print_bytes(&buf[..len]);
    if old_len > len {
        for _ in 0..(old_len - len) {
            print_str(" ");
        }
    }
    print_str("\r");
    print_str(prompt);
    print_bytes(&buf[..idx]);
}

pub fn read_line_raw(prompt: &str, buf: &mut [u8], mask: bool, history: &mut History) -> usize {
    print_str(prompt);
    let mut len = 0;
    let mut idx = 0;
    let mut old_len;

    loop {
        let key = read_key();
        match key {
            Key::Enter => {
                print_str("\n");
                history.offset = 0;
                break;
            }
            Key::Backspace => {
                if idx > 0 {
                    buf.copy_within(idx..len, idx - 1);
                    idx -= 1;
                    old_len = len;
                    len -= 1;
                    buf[len] = 0;
                    if !mask {
                        redraw(prompt, buf, len, idx, old_len);
                    }
                }
            }
            Key::Delete => {
                if !mask && idx < len {
                    buf.copy_within(idx + 1..len, idx);
                    old_len = len;
                    len -= 1;
                    buf[len] = 0;
                    redraw(prompt, buf, len, idx, old_len);
                }
            }
            Key::Left => {
                if !mask && idx > 0 {
                    idx -= 1;
                    redraw(prompt, buf, len, idx, len);
                }
            }
            Key::Right => {
                if !mask && idx < len {
                    idx += 1;
                    redraw(prompt, buf, len, idx, len);
                }
            }
            Key::Home => {
                if !mask && idx > 0 {
                    idx = 0;
                    redraw(prompt, buf, len, idx, len);
                }
            }
            Key::End => {
                if !mask && idx < len {
                    idx = len;
                    redraw(prompt, buf, len, idx, len);
                }
            }
            Key::Up => {
                if !mask {
                    let max_offset = history.count.min(16);
                    if history.count > 0 && history.offset < max_offset {
                        if history.offset == 0 {
                            history.saved_input[..len].copy_from_slice(&buf[..len]);
                            history.saved_len = len;
                        }
                        let h_idx = (history.count - 1 - history.offset) % 16;
                        history.offset += 1;
                        let h_len = history.entries[h_idx]
                            .iter()
                            .position(|&b| b == 0)
                            .unwrap_or(MAX_INPUT_LEN);
                        old_len = len;
                        len = h_len.min(buf.len() - 1);
                        buf[..len].copy_from_slice(&history.entries[h_idx][..len]);
                        idx = len;
                        redraw(prompt, buf, len, idx, old_len);
                    }
                }
            }
            Key::Down => {
                if !mask && history.offset > 0 {
                    old_len = len;
                    if history.offset == 1 {
                        history.offset = 0;
                        len = history.saved_len.min(buf.len() - 1);
                        buf[..len].copy_from_slice(&history.saved_input[..len]);
                        idx = len;
                    } else {
                        history.offset -= 1;
                        let h_idx = (history.count - history.offset) % 16;
                        let h_len = history.entries[h_idx]
                            .iter()
                            .position(|&b| b == 0)
                            .unwrap_or(MAX_INPUT_LEN);
                        len = h_len.min(buf.len() - 1);
                        buf[..len].copy_from_slice(&history.entries[h_idx][..len]);
                        idx = len;
                    }
                    redraw(prompt, buf, len, idx, old_len);
                }
            }
            Key::Tab => {
                if !mask {
                    if idx == 0 || buf[idx - 1] == b' ' {
                        print_str("\n");
                        for cmd in COMMANDS {
                            print_str(cmd);
                            print_str(" ");
                        }
                        print_str("\n");
                        redraw(prompt, buf, len, idx, len);
                    } else {
                        let start = buf[..idx]
                            .iter()
                            .rposition(|&b| b == b' ')
                            .map_or(0, |pos| pos + 1);
                        if let Ok(partial) = core::str::from_utf8(&buf[start..idx]) {
                            let (common_prefix, match_count) = crate::shell::longest_common_prefix(partial);
                            if match_count > 0 {
                                let missing = &common_prefix[partial.len()..];
                                if !missing.is_empty() {
                                    for c in missing.chars() {
                                        if len < buf.len() - 1 {
                                            if idx < len {
                                                buf.copy_within(idx..len, idx + 1);
                                            }
                                            buf[idx] = c as u8;
                                            idx += 1;
                                            len += 1;
                                        }
                                    }
                                } else if match_count > 1 {
                                    print_str("\n");
                                    for cmd in COMMANDS.iter().filter(|&&cmd| cmd.starts_with(partial)) {
                                        print_str(cmd);
                                        print_str(" ");
                                    }
                                    print_str("\n");
                                }
                                redraw(prompt, buf, len, idx, len);
                            }
                        }
                    }
                }
            }
            Key::Char(c) => {
                if len < buf.len() - 1 {
                    if idx < len {
                        buf.copy_within(idx..len, idx + 1);
                    }
                    buf[idx] = c as u8;
                    idx += 1;
                    old_len = len;
                    len += 1;
                    if !mask {
                        redraw(prompt, buf, len, idx, old_len);
                    }
                }
            }
            Key::Unknown => {}
        }
    }

    len
}
