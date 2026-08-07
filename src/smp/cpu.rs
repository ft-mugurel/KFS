use crate::smp::MAX_CPUS;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub struct CpuTable {
    apic_id: [u8; MAX_CPUS],
    online: [AtomicBool; MAX_CPUS],
    count: AtomicUsize,
}

static mut CPU_TABLE: CpuTable = CpuTable {
    apic_id: [0; MAX_CPUS],
    online: [const { AtomicBool::new(false) }; MAX_CPUS],
    count: AtomicUsize::new(0),
};

pub unsafe fn register_cpu(cpu_id: usize, apic_id: u8) {
    CPU_TABLE.apic_id[cpu_id] = apic_id;
    CPU_TABLE.count.fetch_add(1, Ordering::SeqCst);
}

#[allow(dead_code)]
pub unsafe fn cpu_count() -> usize {
    CPU_TABLE.count.load(Ordering::SeqCst)
}

#[allow(dead_code)]
pub unsafe fn mark_online(cpu_id: usize) {
    CPU_TABLE.online[cpu_id].store(true, Ordering::SeqCst);
}

#[allow(dead_code)]
pub unsafe fn mark_offline(cpu_id: usize) {
    CPU_TABLE.online[cpu_id].store(false, Ordering::SeqCst);
}

pub unsafe fn is_online(cpu_id: usize) -> bool {
    CPU_TABLE.online[cpu_id].load(Ordering::SeqCst)
}

pub unsafe fn online_count() -> usize {
    (0..MAX_CPUS).filter(|&i| is_online(i)).count()
}

#[allow(dead_code)]
pub fn apic_id_of(cpu_id: usize) -> u8 {
    unsafe { CPU_TABLE.apic_id[cpu_id] }
}
