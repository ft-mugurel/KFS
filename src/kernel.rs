#![no_std]
#![no_main]

mod debug;
mod gdt;
mod interrupts;
mod paging;
mod panic;
mod printk;
mod shell;
mod signals;
mod spin;
mod startup_config;
mod vga;
mod x86;

use gdt::gdt::load_gdt;
use interrupts::exceptions::init_exceptions;
use interrupts::idt::init_idt;
use interrupts::keyboard::init::init_keyboard;
use interrupts::pic::init_pic;
use interrupts::syscalls::init_syscalls;
use interrupts::task_queue::execute_tasks;
use interrupts::timer::init_timer;
use paging::init::init_paging;
use shell::init::init_shell;
use vga::text_mod::out::init_virtual_screens;
use vga::text_mod::out::set_screen_accepts_input;
use x86::{disable_interrupts, hlt};

#[unsafe(no_mangle)]
pub extern "C" fn kmain(multiboot_magic: u32, multiboot_info_addr: u32) -> ! {
    disable_interrupts();
    init_virtual_screens();
    pr_info!(
        "startup config: vga={}x{}, virtual_screens={}, scrollback={}\n",
        startup_config::vga::WIDTH,
        startup_config::vga::HEIGHT,
        startup_config::vga::VIRTUAL_SCREENS,
        startup_config::vga::SCROLLBACK_LINES,
    );
    load_gdt();
    init_idt();
    init_exceptions();
    unsafe { init_pic() };
    init_paging(multiboot_magic, multiboot_info_addr);
    init_timer();
    init_syscalls();
    init_keyboard();
    init_shell();
    set_screen_accepts_input(startup_config::shell::SCREEN_INDEX, true);
    x86::enable_interrupts();
    pr_info!("Startup complete, entering task loop\n");
    loop {
        execute_tasks();
        hlt();
    }
}
