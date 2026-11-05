#![no_std]
#![no_main]
#![feature(const_trait_impl)]

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
mod log;
mod modules;
mod paging;
mod panic;
mod pipe;
mod process_memory;
mod sched;
mod security;
mod serial;
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

/// # Safety
/// Must ensure that root (/) is loaded
#[unsafe(link_section = ".init.text")]
unsafe fn load_account_records() {
    unsafe {
        if let Ok(passwd) = fs::resolve_path("/etc/passwd", fs::ROOT_NODE)
            && let Ok(buf_ptr) = paging::kmalloc(4096)
        {
            let buffer = &mut *(buf_ptr as *mut [u8; 4096]);
            let passwd_length = (*passwd).size.min(4096) as usize;
            if let Ok(bytes_read) = (*passwd).read(&mut buffer[..passwd_length], 0) {
                let loaded = security::load_passwd(&buffer[..bytes_read]);
                pr_info!("Loaded {} passwd record(s)", loaded);
            } else {
                pr_warn!("Could not read /etc/passwd");
            }
            let _ = paging::kfree(buf_ptr);
        }
        if !security::ensure_root_passwd() {
            pr_warn!("Could not initialize root passwd record");
        }
        let _ = security::persist_passwd();

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
            pr_warn!("Could not read /etc/shadow");
            security::ensure_root_account();
            return;
        };
        let loaded = security::load_accounts(&buffer[..bytes_read]);
        let _ = paging::kfree(buf_ptr);
        pr_info!("Loaded {} account record(s)", loaded);

        if !security::has_root_account() {
            security::ensure_root_account();
            let _ = security::persist_accounts();
            pr_info!("Initialized default root account");
        }
    }
}

late_initcall!(load_account_records);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmain(multiboot_magic: u32, multiboot_info_addr: u32) -> ! {
    sched::init_boot_thread_info(); // before anything (printk included) calls current*()
    unsafe { BOOT_MULTIBOOT_MAGIC = multiboot_magic };
    unsafe { BOOT_MULTIBOOT_INFO_ADDR = multiboot_info_addr };

    x86::disable_interrupts();
    unsafe { serial::init() };

    unsafe { gdt::load_gdt_bsp() };
    interrupts::init_idt();
    interrupts::init_exceptions();
    unsafe { interrupts::init_pic() };

    vga::text_mod::init_virtual_screens();
    unsafe { interrupts::init_keyboard() };
    interrupts::init_timer();
    // --- SMP bring-up ---
    let Some(acpi_info) = acpi::init() else {
        pr_err!("Failed to initialize ACPI");
        loop {
            x86::hlt();
        }
    };
    unsafe { paging::init_paging(BOOT_MULTIBOOT_MAGIC, BOOT_MULTIBOOT_INFO_ADDR) };
    unsafe { sched::init_scheduler_for_cpu(0) }; // BSP = cpu_id 0

    smp::ipi::init_ipi();
    unsafe { smp::lapic::map_lapic(acpi_info.local_apic_addr) };
    unsafe { smp::lapic::enable_local_apic(0) };
    unsafe { x86::enable_interrupts() };
    unsafe { smp::lapic::calibrate() };
    x86::disable_interrupts();
    unsafe { smp::lapic::start_periodic_timer(interrupts::timer::LAPIC_TICK_MS) };
    unsafe { interrupts::timer::mask_pit() };
    smp::cpu::mark_online(0);
    unsafe { smp::trampoline::install_trampoline() };

    // madt.rs puts the BSP in slot 0, so slot index == cpu_id for every core
    for (cpu_id, cpu) in acpi_info.cpus.iter().enumerate() {
        let Some(cpu) = cpu else { continue };
        if cpu.bsp {
            continue; // don't try to SIPI ourselves
        }
        if !unsafe { smp::trampoline::start_ap(cpu.apic_id, cpu_id) } {
            pr_warn!("not starting remaining APs, a late AP still owns the trampoline");
            break;
        }
    }

    initcall::do_initcalls();
    if !fs::root_filesystem_ready() {
        pr_err!("Required root filesystem initialization failed");
        loop {
            x86::hlt();
        }
    }
    unsafe { initcall::free_init_memory() };

    log::handoff_to_userspace();

    if let Err(e) = unsafe { sched::spawn_init_shells() } {
        log::set_direct_screen_output(true);
        pr_err!("Failed to spawn user-space shells: {:?}", e);
    }

    let idle_esp = sched::idle_stack_top(0);
    unsafe { sched::switch_to_idle_stack(idle_esp) };
}
