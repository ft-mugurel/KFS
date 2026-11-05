use crate::smp::MAX_CPUS;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

pub struct CpuTable {
    apic_id: [AtomicU8; MAX_CPUS],
    online: [AtomicBool; MAX_CPUS],
    count: AtomicUsize,
}

static CPU_TABLE: CpuTable = CpuTable {
    apic_id: [const { AtomicU8::new(0) }; MAX_CPUS],
    online: [const { AtomicBool::new(false) }; MAX_CPUS],
    count: AtomicUsize::new(0),
};

pub fn register_cpu(cpu_id: usize, apic_id: u8) {
    CPU_TABLE.apic_id[cpu_id].store(apic_id, Ordering::SeqCst);
    CPU_TABLE.count.fetch_add(1, Ordering::SeqCst);
}

pub fn cpu_count() -> usize {
    CPU_TABLE.count.load(Ordering::SeqCst)
}

pub fn mark_online(cpu_id: usize) {
    CPU_TABLE.online[cpu_id].store(true, Ordering::SeqCst);
}

pub fn is_online(cpu_id: usize) -> bool {
    CPU_TABLE.online[cpu_id].load(Ordering::SeqCst)
}

pub fn online_count() -> usize {
    (0..MAX_CPUS).filter(|&i| is_online(i)).count()
}

pub fn apic_id_of(cpu_id: usize) -> u8 {
    CPU_TABLE.apic_id[cpu_id].load(Ordering::SeqCst)
}

static IDENTITY_MISMATCHES: AtomicUsize = AtomicUsize::new(0);

/// Called by each AP as it comes up: its LAPIC and its ThreadInfo must both
/// agree with the cpu_id it was started as.
pub fn verify_identity(cpu_id: usize, lapic_id: u8, thread_cpu: usize) {
    if apic_id_of(cpu_id) != lapic_id || thread_cpu != cpu_id {
        IDENTITY_MISMATCHES.fetch_add(1, Ordering::SeqCst);
    }
}

pub fn identity_mismatches() -> usize {
    IDENTITY_MISMATCHES.load(Ordering::SeqCst)
}
