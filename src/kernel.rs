#![no_std]
#![no_main]
#![allow(unsafe_op_in_unsafe_fn)]

use crate::startup_config::pic::{MASK_ENABLE_TIMER_KEYBOARD, MASTER_DATA_PORT};

mod acpi;
mod drivers;
mod dump;
mod error;
mod fs;
mod gdt;
mod initcall;
mod interrupts;
mod ipc;
mod locks;
mod paging;
mod panic;
mod pipe;
mod printk;
mod sched;
mod security;
mod serial;
mod signals;
mod smp;
mod startup_config;
mod syscalls;
mod test;
mod tty;
mod utils;
mod vga;
mod waitqueue;
mod x86;

static mut BOOT_MULTIBOOT_MAGIC: u32 = 0;
static mut BOOT_MULTIBOOT_INFO_ADDR: u32 = 0;

#[unsafe(link_section = ".init.text")]
unsafe fn load_account_records() {
    let Ok(shadow) = fs::resolve_path("/etc/shadow", fs::ROOT_NODE) else {
        security::ensure_root_account();
        let _ = security::persist_accounts();
        return;
    };
    let length = (*shadow).size.min(4096) as usize;
    let Ok(buf_ptr) = paging::kmalloc(4096) else {
        security::ensure_root_account();
        return;
    };
    let buffer = &mut *(buf_ptr as *mut [u8; 4096]);
    let Ok(bytes_read) = (*shadow).read(&mut buffer[..length], 0) else {
        let _ = paging::kfree(buf_ptr);
        pr_warn!("Could not read /etc/shadow\n");
        security::ensure_root_account();
        return;
    };
    let loaded = security::load_accounts(&buffer[..bytes_read]);
    let _ = paging::kfree(buf_ptr);
    pr_info!("Loaded {} account record(s)\n", loaded);

    if !security::has_root_account() {
        security::ensure_root_account();
        let _ = security::persist_accounts();
        pr_info!("Initialized default root account\n");
    }
}

late_initcall!(load_account_records);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmain(multiboot_magic: u32, multiboot_info_addr: u32) -> ! {
    BOOT_MULTIBOOT_MAGIC = multiboot_magic;
    BOOT_MULTIBOOT_INFO_ADDR = multiboot_info_addr;

    x86::disable_interrupts();
    serial::init();

    gdt::load_gdt_bsp();
    interrupts::init_idt();
    interrupts::init_exceptions();
    interrupts::init_pic();

    vga::text_mod::init_virtual_screens();
    interrupts::init_keyboard();
    interrupts::init_timer();
    // --- SMP bring-up ---
    let Some(acpi_info) = acpi::init() else {
        pr_err!("Failed to initialize ACPI");
        loop {
            x86::hlt();
        }
    };
    paging::init_paging(BOOT_MULTIBOOT_MAGIC, BOOT_MULTIBOOT_INFO_ADDR);
    sched::init_scheduler_for_cpu(0); // BSP = cpu_id 0

    smp::ipi::init_ipi();
    smp::lapic::map_lapic(acpi_info.local_apic_addr);
    smp::lapic::enable_local_apic(0);
    x86::enable_interrupts();
    smp::lapic::calibrate();
    x86::disable_interrupts();
    smp::lapic::start_periodic_timer(10);
    x86::outb(MASTER_DATA_PORT, MASK_ENABLE_TIMER_KEYBOARD);
    smp::trampoline::install_trampoline();

    let bsp_apic_id = smp::lapic::this_cpu_apic_id();
    let mut next_cpu_id = 1; // cpu_id 0 is reserved for the BSP

    for cpu in acpi_info.cpus.iter().flatten() {
        if cpu.apic_id == bsp_apic_id {
            continue; // don't try to SIPI ourselves
        }
        if next_cpu_id >= smp::MAX_CPUS {
            pr_warn!(
                "MAX_CPUS reached, ignoring extra core apic_id={}\n",
                cpu.apic_id
            );
            break;
        }
        smp::trampoline::start_ap(cpu.apic_id, next_cpu_id);
        next_cpu_id += 1;
    }

    initcall::do_initcalls();
    if !fs::root_filesystem_ready() {
        pr_err!("Required root filesystem initialization failed\n");
        loop {
            x86::hlt();
        }
    }
    initcall::free_init_memory();

    printk::handoff_to_userspace();

    if let Err(e) = sched::spawn_init_shells() {
        printk::set_direct_screen_output(true);
        pr_err!("Failed to spawn user-space shells: {:?}\n", e);
    }

    let idle_esp = sched::idle_stack_top(0);
    sched::switch_to_idle_stack(idle_esp);
}
