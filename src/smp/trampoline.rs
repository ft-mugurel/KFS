use super::lapic;

use crate::gdt;
use crate::interrupts;
use crate::paging;
use crate::sched;
use crate::smp;
use crate::x86;
use core::sync::atomic::{AtomicU32, Ordering};

pub static AP_BOOTED_COUNT: AtomicU32 = AtomicU32::new(0);
const TRAMPOLINE_LOAD_ADDR: u32 = 0x8000;
const TRAMPOLINE_BLOB: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/build/trampoline.bin"));
const PAGE_SIZE: usize = 0x1000;
const TR_DATA_BASE: usize = 0x100; // must match `TIMES 0x100 - ...` in trampoline.asm
const TR_GDT_PTR_OFF: usize = TR_DATA_BASE; // dw+dd = 6 bytes
const TR_CR3_OFF: usize = TR_DATA_BASE + 6; // dd = 4 bytes
const TR_CPU_ID_OFF: usize = TR_DATA_BASE + 10; // dd = 4 bytes
const TR_AP_ENTRY_OFF: usize = TR_DATA_BASE + 14; // dd = 4 bytes
const TR_STACK_TOP_OFF: usize = TR_DATA_BASE + 18; // dd = 4 bytes, last field in the blob

const _: () = assert!(
    TRAMPOLINE_BLOB.len() == TR_STACK_TOP_OFF + 4,
    "trampoline.asm data layout drifted from the TR_*_OFF offsets in trampoline.rs"
);
const _: () = assert!(
    TRAMPOLINE_BLOB.len() <= PAGE_SIZE,
    "trampoline must fit in the single page the SIPI vector points at"
);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ap_entry(cpu_id: u32) -> ! {
    let cpu_id = cpu_id as usize;

    // already on this CPU's idle stack with its ThreadInfo set up by start_ap()
    let idle_esp = unsafe {
        gdt::load_gdt_ap(cpu_id);
        interrupts::init_idt();
        lapic::enable_local_apic(cpu_id);
        smp::cpu::verify_identity(
            cpu_id,
            lapic::this_cpu_apic_id(),
            sched::current_cpu() as usize,
        );
        lapic::start_periodic_timer(crate::interrupts::timer::LAPIC_TICK_MS);
        sched::idle_stack_top(cpu_id)
    };

    AP_BOOTED_COUNT.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
    smp::cpu::mark_online(cpu_id);

    unsafe { sched::switch_to_idle_stack(idle_esp) };
}

#[unsafe(link_section = ".init.text")]
pub unsafe fn install_trampoline() {
    let dst = TRAMPOLINE_LOAD_ADDR as *mut u8;
    let gdt_ptr_bytes = gdt::gdt_pointer_bytes();
    let cr3 = paging::bootstrap_directory_phys_addr();

    unsafe {
        core::ptr::copy_nonoverlapping(TRAMPOLINE_BLOB.as_ptr(), dst, TRAMPOLINE_BLOB.len());
        core::ptr::copy_nonoverlapping(
            gdt_ptr_bytes.as_ptr(),
            dst.add(TR_GDT_PTR_OFF),
            gdt_ptr_bytes.len(),
        );
        core::ptr::write_unaligned(dst.add(TR_CR3_OFF) as *mut u32, cr3);
        core::ptr::write_unaligned(
            dst.add(TR_AP_ENTRY_OFF) as *mut u32,
            ap_entry as *const () as u32,
        );
    }
}

/// Returns false if the AP never checked in. The caller must then stop starting
/// APs: a late AP may still read tr_cpu_id/tr_stack_top, so they must not be
/// overwritten with another CPU's values.
#[unsafe(link_section = ".init.text")]
pub unsafe fn start_ap(apic_id: u8, cpu_id: usize) -> bool {
    // build the idle task and ThreadInfo on the AP's stack before it runs, so
    // current()/current_cpu() are valid from its first instruction
    unsafe { sched::init_scheduler_for_cpu(cpu_id) };

    // tell this specific AP which cpu_id slot and stack it gets before waking it
    let tr = TRAMPOLINE_LOAD_ADDR as usize;
    unsafe {
        core::ptr::write_unaligned((tr + TR_CPU_ID_OFF) as *mut u32, cpu_id as u32);
        core::ptr::write_unaligned(
            (tr + TR_STACK_TOP_OFF) as *mut u32,
            sched::idle_stack_top(cpu_id),
        );
    }

    let vector = (TRAMPOLINE_LOAD_ADDR >> 12) as u8; // SIPI vector = page number

    let before = AP_BOOTED_COUNT.load(Ordering::SeqCst);

    unsafe {
        lapic::send_init_ipi(apic_id);
        x86::busy_wait_us(10_000); // 10ms, per Intel MP spec
        lapic::send_sipi(apic_id, vector);
        x86::busy_wait_us(200);
        lapic::send_sipi(apic_id, vector); // second SIPI required on real hardware; QEMU tolerates skipping but we do it anyway
        x86::busy_wait_us(200);
    }

    let mut waited = 0;
    while AP_BOOTED_COUNT.load(Ordering::SeqCst) == before {
        x86::busy_wait_us(1000);
        waited += 1;
        if waited > 1000 {
            crate::pr_err!("CPU apic_id={} failed to start", apic_id);
            return false;
        }
    }
    true
}
