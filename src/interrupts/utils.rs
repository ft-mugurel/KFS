use crate::x86::{outb, outw};

pub const KEYBOARD_CONTROLLER_COMMAND_PORT: u16 = 0x64;
pub const KEYBOARD_CONTROLLER_REBOOT: u8 = 0xFE;
pub const PCI_RESET_PORT: u16 = 0xCF9;
pub const PCI_RESET_VALUE: u8 = 0x06;
pub const QEMU_SHUTDOWN_PORT: u16 = 0x604;
pub const QEMU_SHUTDOWN_VALUE: u16 = 0x2000;
pub const BOCHS_SHUTDOWN_PORT: u16 = 0xB004;
pub const BOCHS_SHUTDOWN_VALUE: u16 = 0x2000;
pub const VBOX_SHUTDOWN_PORT: u16 = 0x4004;
pub const VBOX_SHUTDOWN_VALUE: u16 = 0x3400;

pub(crate) unsafe fn request_reboot() {
    let _ = crate::log::file_logger::flush_to_file();
    let _ = unsafe { crate::fs::buffer_cache::bsync() };

    unsafe {
        outb(KEYBOARD_CONTROLLER_COMMAND_PORT, KEYBOARD_CONTROLLER_REBOOT);
        outb(PCI_RESET_PORT, PCI_RESET_VALUE);
        outw(QEMU_SHUTDOWN_PORT, QEMU_SHUTDOWN_VALUE);
        outw(BOCHS_SHUTDOWN_PORT, BOCHS_SHUTDOWN_VALUE);
        outw(VBOX_SHUTDOWN_PORT, VBOX_SHUTDOWN_VALUE);
    }
}
