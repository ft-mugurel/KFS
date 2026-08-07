use crate::x86::{outb, outw};

use crate::startup_config::power;

pub(crate) fn request_reboot() {
    outb(
        power::KEYBOARD_CONTROLLER_COMMAND_PORT,
        power::KEYBOARD_CONTROLLER_REBOOT,
    );
    outb(power::PCI_RESET_PORT, power::PCI_RESET_VALUE);
    outw(power::QEMU_SHUTDOWN_PORT, power::QEMU_SHUTDOWN_VALUE);
    outw(power::BOCHS_SHUTDOWN_PORT, power::BOCHS_SHUTDOWN_VALUE);
    outw(power::VBOX_SHUTDOWN_PORT, power::VBOX_SHUTDOWN_VALUE);
}
