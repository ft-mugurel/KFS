#![no_std]
#![no_main]

mod debug;
mod fs;
mod gdt;
mod interrupts;
mod ipc;
mod paging;
mod panic;
mod printk;
mod sched;
mod shell;
mod signals;
mod spin;
mod startup_config;
mod syscalls;
mod test;
mod vga;
mod x86;

use interrupts::exceptions::init_exceptions;
use interrupts::idt::init_idt;
use interrupts::keyboard::init::init_keyboard;
use interrupts::pic::init_pic;
use interrupts::task_queue::execute_tasks;
use interrupts::timer::init_timer;
use paging::init::init_paging;
use sched::scheduler::init_scheduler;
use shell::init::init_shell;
use vga::text_mod::out::init_virtual_screens;
use x86::{disable_interrupts, enable_interrupts, hlt};

pub const USER_PROCESS: unsafe fn() = test::process_fork;

#[unsafe(no_mangle)]
pub extern "C" fn kmain(multiboot_magic: u32, multiboot_info_addr: u32) -> ! {
    disable_interrupts();
    init_timer();
    init_virtual_screens();
    gdt::load_gdt();
    init_idt();
    init_exceptions();
    init_pic();
    init_paging(multiboot_magic, multiboot_info_addr);
    syscalls::init_syscalls();
    init_scheduler();
    init_keyboard();
    init_shell();
    enable_interrupts();
    loop {
        execute_tasks();
        hlt();
    }
}
