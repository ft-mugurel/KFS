use crate::{interrupts::task_queue::schedule_task, x86::{outw, outb}};
use core::arch::asm;

use crate::startup_config::power;

fn execute_shutdown() {
    outw(power::QEMU_SHUTDOWN_PORT, power::QEMU_SHUTDOWN_VALUE); // QEMU
    outw(power::BOCHS_SHUTDOWN_PORT, power::BOCHS_SHUTDOWN_VALUE); // Bochs
    outw(power::VIRTUALBOX_SHUTDOWN_PORT, power::VIRTUALBOX_SHUTDOWN_VALUE); // VirtualBox

    loop {
        unsafe { asm!("hlt") };
    }
}

pub(crate) fn request_shutdown() {
    schedule_task(execute_shutdown);
}

pub(crate) fn request_reboot() {
    outb(power::KEYBOARD_CONTROLLER_COMMAND_PORT, power::KEYBOARD_CONTROLLER_REBOOT);
    outb(power::PCI_RESET_PORT, power::PCI_RESET_VALUE);
    outw(power::QEMU_SHUTDOWN_PORT, power::QEMU_SHUTDOWN_VALUE);
}
