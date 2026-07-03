#![no_std]
#![no_main]

mod drivers;
mod dump;
mod error;
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
mod utils;
mod vga;
mod x86;

use vga::text_mod::init_virtual_screens;

pub const USER_PROCESS: unsafe fn() = test::process_fork;

#[unsafe(no_mangle)]
pub extern "C" fn kmain(multiboot_magic: u32, multiboot_info_addr: u32) -> ! {
    x86::disable_interrupts();
    interrupts::init_timer();
    init_virtual_screens();
    gdt::load_gdt();
    interrupts::init_idt();
    interrupts::init_exceptions();
    interrupts::init_pic();
    paging::init_paging(multiboot_magic, multiboot_info_addr);
    syscalls::init_syscalls();
    sched::init_scheduler();
    interrupts::init_keyboard();
    shell::init_shell();
    x86::enable_interrupts();
    loop {
        sched::execute_tasks();
        x86::hlt();
    }
}
