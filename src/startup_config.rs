pub mod vga {
    pub const BUFFER_ADDR: usize = 0xB8000;
    pub const WIDTH: usize = 80;
    pub const HEIGHT: usize = 25;
    pub const STATUS_BAR_LINES: usize = 1;
    pub const CONTENT_HEIGHT: usize = HEIGHT - STATUS_BAR_LINES;

    pub const VIRTUAL_SCREENS: usize = 6;
    pub const SCROLLBACK_LINES: usize = 200;
}

pub mod shell {
    pub const SCREEN_INDEX: usize = 0;
    pub const MAX_INPUT_LEN: usize = 128;
}

pub mod logging {
    pub const DEFAULT_LOG_SCREEN: usize = 1;
    pub const DEFAULT_DEBUG_LOG_SCREEN: usize = 2;
}

pub mod idt {
    pub const ENTRIES: usize = 256; // Max number of IDT entries cpu can have
}

pub mod pic {
    pub const MASTER_COMMAND_PORT: u16 = 0x20;
    pub const MASTER_DATA_PORT: u16 = 0x21;
    pub const SLAVE_COMMAND_PORT: u16 = 0xA0;
    pub const SLAVE_DATA_PORT: u16 = 0xA1;
    pub const PIT_COMMAND_PORT: u16 = 0x43;
    pub const PIT_CHANNEL0_PORT: u16 = 0x40;
    pub const KEYBOARD_DATA_PORT: u16 = 0x60;
    pub const KEYBOARD_COMMAND_PORT: u16 = 0x64;

    pub const ICW1_INIT: u8 = 0x11;
    pub const ICW4_8086: u8 = 0x01;

    pub const MASTER_IRQ_OFFSET: u8 = 0x20;
    pub const SLAVE_IRQ_OFFSET: u8 = 0x28;
    pub const SLAVE_CONNECTED_TO_IRQ: u8 = 0x04;
    pub const CASCADE_IDENTITY: u8 = 0x02;

    pub const MASK_IRQ0: u8 = 1 << 0; // timer
    pub const MASK_IRQ1: u8 = 1 << 1; // keyboard
    pub const MASK_ENABLE_TIMER_KEYBOARD: u8 = !(MASK_IRQ0 | MASK_IRQ1);
    pub const MASK_ALL: u8 = 0xFF;
    pub const EOI: u8 = 0x20;

    pub const KEYBOARD_IRQ_LINE: u8 = 1;
    pub const TIMER_IRQ_VECTOR: u8 = MASTER_IRQ_OFFSET + 0;
    pub const KEYBOARD_IRQ_VECTOR: u8 = MASTER_IRQ_OFFSET + KEYBOARD_IRQ_LINE;
}

pub mod power {
    pub const QEMU_SHUTDOWN_PORT: u16 = 0x604;
    pub const QEMU_SHUTDOWN_VALUE: u16 = 0x2000;

    pub const BOCHS_SHUTDOWN_PORT: u16 = 0xB004;
    pub const BOCHS_SHUTDOWN_VALUE: u16 = 0x2000;

    pub const VBOX_SHUTDOWN_PORT: u16 = 0x4004;
    pub const VBOX_SHUTDOWN_VALUE: u16 = 0x3400;

    pub const KEYBOARD_CONTROLLER_COMMAND_PORT: u16 = 0x64;
    pub const KEYBOARD_CONTROLLER_REBOOT: u8 = 0xFE;

    pub const PCI_RESET_PORT: u16 = 0xCF9;
    pub const PCI_RESET_VALUE: u8 = 0x06;

    pub const CONFIG_HZ: u32 = 1000;
}
