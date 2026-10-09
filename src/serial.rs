use core::fmt::{self, Write};
use crate::locks::Spinlock;

pub static SERIAL_LOCK: Spinlock<()> = Spinlock::new(());
pub const COM1_PORT: u16 = 0x3F8;

#[unsafe(link_section = ".init.text")]
pub fn init() {
    crate::x86::outb(COM1_PORT + 1, 0x00); // Disable all interrupts
    crate::x86::outb(COM1_PORT + 3, 0x80); // Enable DLAB (set baud rate divisor)
    crate::x86::outb(COM1_PORT + 0, 0x03); // Set divisor to 3 (lo byte) 38400 baud
    crate::x86::outb(COM1_PORT + 1, 0x00); //                  (hi byte)
    crate::x86::outb(COM1_PORT + 3, 0x03); // 8 bits, no parity, one stop bit
    crate::x86::outb(COM1_PORT + 2, 0xC7); // Enable FIFO, clear them, with 14-byte threshold
    crate::x86::outb(COM1_PORT + 4, 0x0B); // IRQs enabled, RTS/DSR set
}

#[inline]
pub fn write_byte_unlocked(byte: u8) {
    if byte == b'\n' {
        for _ in 0..1000 {
            if (crate::x86::inb(COM1_PORT + 5) & 0x20) != 0 {
                break;
            }
        }
        crate::x86::outb(COM1_PORT, b'\r');
    }
    for _ in 0..1000 {
        if (crate::x86::inb(COM1_PORT + 5) & 0x20) != 0 {
            break;
        }
    }
    crate::x86::outb(COM1_PORT, byte);
}

pub fn write_str_unlocked(s: &str) {
    for &b in s.as_bytes() {
        write_byte_unlocked(b);
    }
}

struct SerialWriter;

impl Write for SerialWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_str_unlocked(s);
        Ok(())
    }
}

pub fn write_fmt_unlocked(args: fmt::Arguments<'_>) {
    let mut writer = SerialWriter;
    let _ = writer.write_fmt(args);
}

#[allow(dead_code)]
pub fn write_str(s: &str) {
    let _guard = SERIAL_LOCK.lock();
    write_str_unlocked(s);
}

#[allow(dead_code)]
pub fn write_fmt(args: fmt::Arguments<'_>) {
    let _guard = SERIAL_LOCK.lock();
    write_fmt_unlocked(args);
}

pub fn write_printk_to_serial(level_tag: &str, pid: u32, args: &fmt::Arguments<'_>) {
    let _guard = SERIAL_LOCK.lock();
    write_str_unlocked(level_tag);
    write_fmt_unlocked(format_args!("({}) ", pid));
    write_fmt_unlocked(*args);
}
