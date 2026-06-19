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
    
    loop {
        unsafe {
            let msg: [u8; 9] = [b'A', b'l', b'i', b'v', b'e', b'.', b'.', b'.', b'\n'];
            // Print "Alive..."
            core::arch::asm!(
                "int 0x80",
                in("eax") 4,
                in("ebx") 1,
                in("ecx") msg.as_ptr(),
                in("edx") msg.len(),
                options(nostack),
            );
            core::arch::asm!(
                "int 0x80",
                in("eax") 162,
                in("ebx") 2000,
                options(nostack, nomem),
            );
        }
    }
}

/* fn user_process_ign() {
    unsafe {
        let mut heap_ptr: u32;
        core::arch::asm!(
            "int 0x80",
            in("eax") 45,
            in("ebx") 100,
            lateout("eax") heap_ptr,
            options(nostack, nomem),
        );

        let str_bytes: [u8; 24] = [
            b'D', b'y', b'n', b'a', b'm', b'i', b'c', b' ', b'H', b'e', b'a', b'p', b' ', b'M',
            b'e', b'm', b'o', b'r', b'y', b' ', b'O', b'K', b'!', b'\n',
        ];
        let ptr = heap_ptr as *mut u8;
        for i in 0..str_bytes.len() {
            *ptr.add(i) = str_bytes[i];
        }

        core::arch::asm!(
            "int 0x80",
            in("eax") 4,
            in("ebx") 1,
            in("ecx") heap_ptr,
            in("edx") str_bytes.len(),
            options(nostack, nomem),
        );

        // 4. Exit safely
        core::arch::asm!("int 0x80", in("eax") 1, in("ebx") 0, options(noreturn));
    }
}
 */
#[unsafe(no_mangle)]
pub extern "C" fn kmain(multiboot_magic: u32, multiboot_info_addr: u32) -> ! {
    disable_interrupts();
    init_virtual_screens();
    load_gdt();
    init_idt();
    init_exceptions();
    init_pic();
    init_paging(multiboot_magic, multiboot_info_addr);
    init_syscalls();
    init_scheduler();
    init_timer();
    init_keyboard();
    init_shell();
    enable_interrupts();
    loop {
        execute_tasks();
        hlt();
    }
}
