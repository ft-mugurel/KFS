use super::lapic;

use crate::gdt;
use crate::interrupts;
use crate::paging;
use crate::sched;
use crate::x86;
use core::sync::atomic::{AtomicU32, Ordering};

pub static AP_BOOTED_COUNT: AtomicU32 = AtomicU32::new(0);
const TRAMPOLINE_LOAD_ADDR: u32 = 0x8000;
static TRAMPOLINE_BLOB: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/build/trampoline.bin"));
const PAGE_SIZE: usize = 0x1000;
const TR_STACK_SIZE: usize = 4096; // must match STACK_SIZE in trampoline.asm
const TR_TMP_STACK_OFF: usize = TR_AP_ENTRY_OFF + 4; // stack array starts right after tr_ap_entry_addr
const TR_DATA_BASE: usize = 0x100; // must match `TIMES 0x100 - ...` in trampoline.asm
const TR_GDT_PTR_OFF: usize = TR_DATA_BASE; // dw+dd = 6 bytes
const TR_CR3_OFF: usize = TR_DATA_BASE + 6; // dd = 4 bytes
const TR_CPU_ID_OFF: usize = TR_DATA_BASE + 10; // dd = 4 bytes
const TR_AP_ENTRY_OFF: usize = TR_DATA_BASE + 14;

#[unsafe(no_mangle)]
#[unsafe(link_section = ".init.text")]
pub unsafe extern "C" fn ap_entry(cpu_id: u32) -> ! {
    let cpu_id = cpu_id as usize;

    gdt::load_gdt_ap(cpu_id);
    sched::init_scheduler_for_cpu(cpu_id);
    interrupts::init_idt();
    lapic::enable_local_apic(cpu_id);

    AP_BOOTED_COUNT.fetch_add(1, core::sync::atomic::Ordering::SeqCst);

    lapic::calibrate();
    lapic::start_periodic_timer(10);
    let idle_esp = sched::idle_stack_top(cpu_id);
    sched::switch_to_idle_stack(idle_esp);
}

#[unsafe(link_section = ".init.text")]
pub unsafe fn install_trampoline() {
    let required_len = TR_TMP_STACK_OFF + (super::MAX_CPUS * TR_STACK_SIZE);
    let pages_needed = (required_len + PAGE_SIZE - 1) / PAGE_SIZE;

    assert!(
        TRAMPOLINE_BLOB.len() >= required_len,
        "trampoline blob ({} bytes) is smaller than the layout it's supposed to contain \
         ({} bytes) — asm and Rust offsets have drifted",
        TRAMPOLINE_BLOB.len(),
        required_len
    );
    assert!(
        TRAMPOLINE_BLOB.len() <= pages_needed * PAGE_SIZE,
        "trampoline blob spans {} pages unexpectedly — check for padding/alignment drift",
        pages_needed
    );
    assert!(
        TR_CPU_ID_OFF + 4 <= TRAMPOLINE_BLOB.len(),
        "trampoline layout drifted from Rust offsets"
    );
    assert!(
        TR_TMP_STACK_OFF + (super::MAX_CPUS * TR_STACK_SIZE) <= TRAMPOLINE_BLOB.len(),
        "trampoline's per-AP stack region doesn't cover smp::MAX_CPUS cores — \
         asm's MAX_CPUS/STACK_SIZE and Rust's smp::MAX_CPUS have drifted out of sync"
    );

    let dst = TRAMPOLINE_LOAD_ADDR as *mut u8;
    core::ptr::copy_nonoverlapping(TRAMPOLINE_BLOB.as_ptr(), dst, TRAMPOLINE_BLOB.len());

    let gdt_ptr_bytes = gdt::gdt_pointer_bytes();
    core::ptr::copy_nonoverlapping(
        gdt_ptr_bytes.as_ptr(),
        dst.add(TR_GDT_PTR_OFF),
        gdt_ptr_bytes.len(),
    );

    let cr3 = paging::bootstrap_directory_phys_addr();
    core::ptr::write_unaligned(dst.add(TR_CR3_OFF) as *mut u32, cr3);

    core::ptr::write_unaligned(
        dst.add(TR_AP_ENTRY_OFF) as *mut u32,
        ap_entry as *const () as u32,
    );
}

#[unsafe(link_section = ".init.text")]
pub unsafe fn start_ap(apic_id: u8, cpu_id: usize) {
    // tell this specific AP which cpu_id slot it is before waking it
    core::ptr::write_unaligned(
        (TRAMPOLINE_LOAD_ADDR as usize + TR_CPU_ID_OFF) as *mut u32,
        cpu_id as u32,
    );

    let vector = (TRAMPOLINE_LOAD_ADDR >> 12) as u8; // SIPI vector = page number

    let before = AP_BOOTED_COUNT.load(Ordering::SeqCst);

    lapic::send_init_ipi(apic_id);
    x86::busy_wait_us(10_000); // 10ms, per Intel MP spec
    lapic::send_sipi(apic_id, vector);
    x86::busy_wait_us(200);
    lapic::send_sipi(apic_id, vector); // second SIPI required on real hardware; QEMU tolerates skipping but we do it anyway
    x86::busy_wait_us(200);

    let mut waited = 0;
    while AP_BOOTED_COUNT.load(Ordering::SeqCst) == before {
        x86::busy_wait_us(1000);
        waited += 1;
        if waited > 1000 {
            crate::pr_err!("CPU apic_id={} failed to start\n", apic_id);
            return;
        }
    }
}
