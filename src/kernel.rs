#![no_std]
#![no_main]

use crate::startup_config::pic::{MASK_ENABLE_TIMER_KEYBOARD, MASTER_DATA_PORT};

mod drivers;
mod dump;
mod error;
mod fs;
mod gdt;
// mod initcall;
mod acpi;
mod interrupts;
mod ipc;
mod locks;
mod paging;
mod panic;
mod printk;
mod sched;
mod security;
mod shell;
mod signals;
mod smp;
mod startup_config;
mod syscalls;
mod test;
mod tty;
mod utils;
mod vga;
mod x86;

static mut BOOT_MULTIBOOT_MAGIC: u32 = 0;
static mut BOOT_MULTIBOOT_INFO_ADDR: u32 = 0;

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

/* unsafe fn free_init_memory() {
    let start_addr = core::ptr::addr_of!(__init_start) as u32;
    let end_addr = core::ptr::addr_of!(__init_end) as u32;

    let page_size = 4096;
    let mut current_addr = start_addr;

    crate::pr_info!(
        "Freeing init memory: {:#X} - {:#X} ({} bytes)\n",
        start_addr,
        end_addr,
        end_addr - start_addr
    );

    // Ensure we only free full pages
    while current_addr < end_addr {
        let Some(phys_addr) = paging::virt_to_phys(current_addr) else {
            crate::pr_err!(
                "Failed to convert virtual address to physical: {:#X}\n",
                current_addr
            );
            break;
        };

        paging::free_physical_page(phys_addr)
            .consume_err("Failed to free physical page for init memory");

        paging::unmap_page(current_addr).consume_err("Failed to unmap page for init memory");

        current_addr += page_size;
    }
} */

fn prepare_serial_output() {
    x86::outb(0x3F8 + 1, 0x00); // Disable all interrupts
    x86::outb(0x3F8 + 3, 0x80); // Enable DLAB (set baud rate divisor)
    x86::outb(0x3F8 + 0, 0x03); // Set divisor to 3 (lo byte) 38400 baud
    x86::outb(0x3F8 + 1, 0x00); //                  (hi byte)
    x86::outb(0x3F8 + 3, 0x03); // 8 bits, no parity, one stop bit
    x86::outb(0x3F8 + 2, 0xC7); // Enable FIFO, clear them, with 14-byte threshold
    x86::outb(0x3F8 + 4, 0x0B); // IRQs enabled, RTS/DSR set
}

#[no_mangle]
pub unsafe extern "C" fn kmain(multiboot_magic: u32, multiboot_info_addr: u32) -> ! {
    BOOT_MULTIBOOT_MAGIC = multiboot_magic;
    BOOT_MULTIBOOT_INFO_ADDR = multiboot_info_addr;

    x86::disable_interrupts();
    prepare_serial_output();

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
    let cr = sched::current();
    pr_info!(
        "Current task struct for CPU 0: {:#X}\n",
        cr.as_ref().map_or(0, |t| t as *const _ as u32)
    );

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

    let _ = drivers::init();
    syscalls::init_syscalls();

    if let Some(device_id) = drivers::first_ext2_partition() {
        let _ = fs::ext2::mount_device(device_id);
    } else {
        pr_warn!("No EXT2 partition found for the root filesystem\n");
    }
    load_account_records();
    if let Err(error) = tty::init() {
        pr_err!("Failed to initialize virtual terminals: {:?}\n", error);
    }
    shell::init_shell();
    test::fs_boot_probe();

    let idle_esp = sched::idle_stack_top(0);
    sched::switch_to_idle_stack(idle_esp);
}
