// smp/ipi.rs
use crate::{
    interrupts, pr_warn,
    smp::lapic,
    x86::{self, outw},
};
use core::sync::atomic::{AtomicU32, Ordering};

pub const SHUTDOWN_VECTOR: u8 = 0xFE;
static PARKED_CORES: AtomicU32 = AtomicU32::new(0);

pub fn init_ipi() {
    interrupts::register_interrupt_handler(SHUTDOWN_VECTOR, shutdown_ipi_handler);
}

pub unsafe fn send_shutdown_ipi_to_others() {
    lapic::send_ipi_all_excluding_self(SHUTDOWN_VECTOR);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn shutdown_ipi_handler() {
    PARKED_CORES.fetch_add(1, Ordering::SeqCst);
    lapic::send_eoi();
    x86::disable_interrupts();
    loop {
        x86::hlt();
    }
}

pub unsafe fn request_shutdown() {
    send_shutdown_ipi_to_others();

    let expected = super::cpu::online_count() as u32 - 1;
    let mut waited = 0u32;
    while PARKED_CORES.load(core::sync::atomic::Ordering::SeqCst) < expected && waited < 1_000_000 {
        waited += 1;
    }
    if waited >= 1_000_000 {
        pr_warn!("shutdown: not all cores parked in time, powering off anyway\n");
    }

    x86::disable_interrupts();
    outw(0x604, 0x2000);
}
