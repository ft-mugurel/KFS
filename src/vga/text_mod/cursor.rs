use super::{SCREEN_CONTENT_HEIGHT, VGA_HEIGHT, VGA_WIDTH, VirtualScreen, screen};
use crate::x86::{inb, outb};

const VGA_CMD_PORT: u16 = 0x3D4;
const VGA_DATA_PORT: u16 = 0x3D5;

pub fn set_big_cursor() {
    unsafe { write_cursor_shape(0x00, 0x0F) };
}

pub fn set_small_cursor() {
    unsafe { write_cursor_shape(0x0E, 0x0F) };
}

pub fn set_cursor_shape(start: u8, end: u8) {
    unsafe { write_cursor_shape(start, end) };
}

pub unsafe fn set_cursor(x: u16, y: u16) {
    let max_x = (VGA_WIDTH - 1) as u16;
    let max_y = (VGA_HEIGHT - 1) as u16;
    let x = x.min(max_x);
    let y = y.min(max_y);
    let position = y * VGA_WIDTH as u16 + x;
    unsafe {
        outb(VGA_CMD_PORT, 0x0E);
        outb(VGA_DATA_PORT, (position >> 8) as u8);
        outb(VGA_CMD_PORT, 0x0F);
        outb(VGA_DATA_PORT, (position & 0xFF) as u8);
    }
}

pub fn disable_cursor() {
    unsafe {
        outb(VGA_CMD_PORT, 0x0A);
        let cursor_start = inb(VGA_DATA_PORT);
        outb(VGA_DATA_PORT, cursor_start | 0x20);
    }
}

pub fn enable_cursor() {
    unsafe {
        outb(VGA_CMD_PORT, 0x0A);
        let cursor_start = inb(VGA_DATA_PORT);
        outb(VGA_DATA_PORT, cursor_start & !0x20);
    }
}
unsafe fn write_cursor_shape(start: u8, end: u8) {
    unsafe {
        outb(VGA_CMD_PORT, 0x0A);
        let cursor_start = inb(VGA_DATA_PORT);
        outb(VGA_DATA_PORT, (cursor_start & 0xE0) | (start & 0x1F));

        outb(VGA_CMD_PORT, 0x0B);
        let cursor_end = inb(VGA_DATA_PORT);
        outb(VGA_DATA_PORT, (cursor_end & 0xE0) | (end & 0x1F));
    }
}

pub fn sync_hardware_cursor(screen: &VirtualScreen) {
    if !screen.cursor_visible {
        disable_cursor();
        return;
    }

    let cursor_x = usize::from(screen.cursor.x);
    let cursor_y = usize::from(screen.cursor.y);
    let top_line = screen::visible_top_line_of(screen);

    unsafe {
        if cursor_y >= top_line && cursor_y < top_line + SCREEN_CONTENT_HEIGHT {
            enable_cursor();
            set_cursor(
                cursor_x.min(VGA_WIDTH - 1) as u16,
                (cursor_y - top_line + 1) as u16,
            );
        } else {
            disable_cursor();
        }
    }
}
