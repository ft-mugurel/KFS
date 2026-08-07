use crate::paging::{self, PAGE_PCD, PAGE_PRESENT, PAGE_WRITABLE};
use crate::x86::{rdmsr, wrmsr};
use core::sync::atomic::{AtomicU32, Ordering};

const IA32_APIC_BASE_MSR: u32 = 0x1B;
const APIC_BASE_ENABLE: u64 = 1 << 11;

const REG_ID: u32 = 0x20;
const REG_TPR: u32 = 0x80;
const REG_SVR: u32 = 0xF0;
const REG_LVT_LINT0: u32 = 0x350;
const REG_LVT_LINT1: u32 = 0x360;

const SVR_APIC_ENABLE: u32 = 1 << 8;
const SPURIOUS_VECTOR: u32 = 0xFF; // conventionally the last usable vector
const LVT_DELIVERY_EXTINT: u32 = 0b111 << 8; // relay whatever the PIC is asserting
const LVT_DELIVERY_NMI: u32 = 0b100 << 8;
const LVT_MASKED: u32 = 1 << 16;

static LAPIC_BASE: AtomicU32 = AtomicU32::new(0);

/// Called once by the BSP, after ACPI/MADT parsing gives us the physical base.
/// Physical == virtual here (identity mapping), since page tables are shared
/// across all cores — every core sees this same mapping once CR3 is loaded.
pub unsafe fn map_lapic(phys_base: u32) {
    paging::map_page(
        phys_base,
        phys_base,
        PAGE_PRESENT | PAGE_WRITABLE | PAGE_PCD,
    )
    .expect("failed to map LAPIC MMIO region");
    LAPIC_BASE.store(phys_base, Ordering::SeqCst);
}

unsafe fn reg_write(reg: u32, value: u32) {
    let base = LAPIC_BASE.load(Ordering::SeqCst);
    core::ptr::write_volatile((base + reg) as *mut u32, value);
}

unsafe fn reg_read(reg: u32) -> u32 {
    let base = LAPIC_BASE.load(Ordering::SeqCst);
    core::ptr::read_volatile((base + reg) as *const u32)
}

/// Called by every core (BSP once in kmain, each AP once from ap_entry)
/// to switch its own local APIC on.
pub unsafe fn enable_local_apic(cpu_id: usize) {
    // Make sure the MSR-level enable bit is set
    let base = rdmsr(IA32_APIC_BASE_MSR);
    wrmsr(IA32_APIC_BASE_MSR, base | APIC_BASE_ENABLE);

    reg_write(REG_TPR, 0); // accept all interrupt priorities
    reg_write(REG_SVR, SVR_APIC_ENABLE | SPURIOUS_VECTOR);

    if cpu_id == 0 {
        reg_write(REG_LVT_LINT0, LVT_DELIVERY_EXTINT);
    } else {
        reg_write(REG_LVT_LINT0, LVT_MASKED);
    }
    reg_write(REG_LVT_LINT1, LVT_DELIVERY_NMI | LVT_MASKED);
}

pub unsafe fn this_cpu_apic_id() -> u8 {
    (reg_read(REG_ID) >> 24) as u8
}

const ICR_LOW: u32 = 0x300;
const ICR_HIGH: u32 = 0x310;

unsafe fn lapic_write(reg: u32, value: u32) {
    LAPIC_BASE.load(Ordering::SeqCst);
    let base = LAPIC_BASE.load(Ordering::SeqCst);
    core::ptr::write_volatile((base + reg) as *mut u32, value);
}
unsafe fn lapic_read(reg: u32) -> u32 {
    LAPIC_BASE.load(Ordering::SeqCst);
    let base = LAPIC_BASE.load(Ordering::SeqCst);
    core::ptr::read_volatile((base + reg) as *const u32)
}

pub unsafe fn send_init_ipi(apic_id: u8) {
    lapic_write(ICR_HIGH, (apic_id as u32) << 24);
    lapic_write(ICR_LOW, 0x4500); // INIT, edge, assert, physical destination
    while lapic_read(ICR_LOW) & (1 << 12) != 0 {} // wait for delivery
}

pub unsafe fn send_sipi(apic_id: u8, vector: u8) {
    lapic_write(ICR_HIGH, (apic_id as u32) << 24);
    lapic_write(ICR_LOW, 0x4600 | vector as u32); // Startup IPI
    while lapic_read(ICR_LOW) & (1 << 12) != 0 {}
}

pub unsafe fn send_eoi() {
    lapic_write(0xB0, 0);
}

pub unsafe fn send_ipi_all_excluding_self(vector: u8) {
    reg_write(ICR_HIGH, 0);
    reg_write(ICR_LOW, (0b11 << 18) | vector as u32); // fixed delivery, all-excl-self shorthand
}
