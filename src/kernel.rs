#![no_std]
#![no_main]

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
mod shell;
mod signals;
mod smp;
mod startup_config;
mod syscalls;
mod test;
mod utils;
mod vga;
mod x86;

static mut BOOT_MULTIBOOT_MAGIC: u32 = 0;
static mut BOOT_MULTIBOOT_INFO_ADDR: u32 = 0;

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

#[no_mangle]
pub unsafe extern "C" fn kmain(multiboot_magic: u32, multiboot_info_addr: u32) -> ! {
    BOOT_MULTIBOOT_MAGIC = multiboot_magic;
    BOOT_MULTIBOOT_INFO_ADDR = multiboot_info_addr;

    x86::disable_interrupts();
    gdt::load_gdt_bsp();
    interrupts::init_idt();
    interrupts::init_exceptions();
    interrupts::init_pic();

    vga::text_mod::init_virtual_screens();

    interrupts::init_timer();
    // --- SMP bring-up ---
    let Some(acpi_info) = acpi::init() else {
        pr_err!("Failed to initialize ACPI");
        loop {
            x86::hlt();
        }
    };
    paging::init_paging(BOOT_MULTIBOOT_MAGIC, BOOT_MULTIBOOT_INFO_ADDR);
    // not sure where to put this
    // test::run_memory_tests();
    sched::init_scheduler_for_cpu(0); // BSP = cpu_id 0

    smp::ipi::init_ipi();
    smp::lapic::map_lapic(acpi_info.local_apic_addr);
    smp::lapic::enable_local_apic(0);

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

    let _ = fs::ext2::mount_device(0);
    test::fs_boot_probe();
    interrupts::init_keyboard();
    shell::init_shell();

    let idle_esp = sched::idle_stack_top(0);
    sched::switch_to_idle_stack(idle_esp);
}
