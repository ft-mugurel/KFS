#![no_std]
#![no_main]

mod debug;
mod gdt;
mod interrupts;
mod paging;
mod panic;
mod printk;
mod sched;
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
use sched::scheduler::init_scheduler;
use shell::init::init_shell;
use vga::text_mod::out::init_virtual_screens;
use x86::{disable_interrupts, enable_interrupts, hlt};

fn user_process() {
    let message: [u8; 11] = [
        b'U', b's', b'e', b'r', b' ', b'S', b'p', b'a', b'c', b'e', b'\n',
    ];

    for _ in 0..5 {
        unsafe {
            core::arch::asm!(
                "int 0x80",
                in("eax") 4,
                in("ebx") 1,
                in("ecx") message.as_ptr(),
                in("edx") message.len(),
                options(nostack, nomem),
            );

            core::arch::asm!(
                "int 0x80",
                in("eax") 162, in("ebx") 1000,
                options(nostack, nomem),
            );
        }
    }

    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("eax") 1,
            in("ebx") 0,
            options(noreturn),
        );
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn kmain(multiboot_magic: u32, multiboot_info_addr: u32) -> ! {
    disable_interrupts();
    init_virtual_screens();
    crate::printk_level_on!(1, crate::printk::printk::KernelLogLevel::Warning, "kmain 1\n"); // Breadcrumb 1
    load_gdt();
    crate::printk_level_on!(1, crate::printk::printk::KernelLogLevel::Warning, "kmain 2\n"); // Breadcrumb 2
    init_idt();
    crate::printk_level_on!(1, crate::printk::printk::KernelLogLevel::Warning, "kmain 3\n"); // Breadcrumb 3
    init_exceptions();
    crate::printk_level_on!(1, crate::printk::printk::KernelLogLevel::Warning, "kmain 4\n"); // Breadcrumb 4
    init_pic();
    crate::printk_level_on!(1, crate::printk::printk::KernelLogLevel::Warning, "kmain 5\n"); // Breadcrumb 5
    init_paging(multiboot_magic, multiboot_info_addr);
    crate::printk_level_on!(1, crate::printk::printk::KernelLogLevel::Warning, "kmain 6\n"); // Breadcrumb 6
    init_syscalls();
    crate::printk_level_on!(1, crate::printk::printk::KernelLogLevel::Warning, "kmain 7\n"); // Breadcrumb 7
    init_scheduler();
    crate::printk_level_on!(1, crate::printk::printk::KernelLogLevel::Warning, "kmain 8\n"); // Breadcrumb 8
    init_timer();
    crate::printk_level_on!(1, crate::printk::printk::KernelLogLevel::Warning, "kmain 9\n"); // Breadcrumb 9
    init_keyboard();
    crate::printk_level_on!(1, crate::printk::printk::KernelLogLevel::Warning, "kmain 10\n"); // Breadcrumb 10
    init_shell();
    crate::printk_level_on!(1, crate::printk::printk::KernelLogLevel::Warning, "kmain 11\n"); // Breadcrumb 11
    enable_interrupts();
    crate::printk_level_on!(1, crate::printk::printk::KernelLogLevel::Warning, "kmain 12\n"); // Breadcrumb 12
    loop {
        execute_tasks();
        hlt();
    }
}
