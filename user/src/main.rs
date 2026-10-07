#![no_std]
#![no_main]

mod io;
mod login;
mod shell;
mod syscall;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    io::print_str("\n[PANIC in user-space shell]\n");
    syscall::sys_exit(1);
}

static mut HISTORY: shell::History = shell::History::new();

#[unsafe(no_mangle)]
#[unsafe(link_section = ".text._start")]
pub extern "C" fn _start() -> ! {
    loop {
        let (uname_buf, uname_len) = unsafe {
            login::tty_login(&mut *core::ptr::addr_of_mut!(HISTORY))
        };
        let uname = core::str::from_utf8(&uname_buf[..uname_len]).unwrap_or("user");
        unsafe {
            shell::run_shell(uname, &mut *core::ptr::addr_of_mut!(HISTORY));
        }
    }
}
