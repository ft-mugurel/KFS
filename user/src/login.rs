// =============================================================================
// KFS User-Space TTY Login Manager
// =============================================================================

use crate::io::{print_str, read_line_raw};
use crate::shell::History;
use crate::syscall::sys_login;

pub fn tty_login(history: &mut History) -> ([u8; 32], usize) {
    let mut user_buf = [0u8; 64];
    let mut pass_buf = [0u8; 64];
    let mut tty_buf = [0u8; 16];

    let tty_len = crate::syscall::sys_getttyname(0, &mut tty_buf);
    let tty_name = if tty_len > 0 {
        core::str::from_utf8(&tty_buf[..tty_len as usize]).unwrap_or("tty1")
    } else {
        "tty1"
    };

    print_str("\x1b[2J\x1b[H");
    print_str("\n\nKFS (Kernel From Scratch) 32-bit x86 (");
    print_str(tty_name);
    print_str(")\n\n");

    loop {
        let ulen = read_line_raw("login: ", &mut user_buf, false, history);
        let username = core::str::from_utf8(&user_buf[..ulen]).unwrap_or("").trim();
        if username.is_empty() {
            continue;
        }

        let plen = read_line_raw("Password: ", &mut pass_buf, true, history);
        let password = &pass_buf[..plen];

        if sys_login(username.as_bytes(), password) == 0 {
            print_str("\n");
            let mut name_out = [0u8; 32];
            let copy_len = username.len().min(32);
            name_out[..copy_len].copy_from_slice(&username.as_bytes()[..copy_len]);
            return (name_out, copy_len);
        } else {
            print_str("Login incorrect\n");
        }
    }
}
