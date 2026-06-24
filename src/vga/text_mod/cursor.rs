use super::{
    screen, CursorMovement, VirtualScreen, SCREEN_CONTENT_HEIGHT, SCROLLBACK_LINES, VGA_HEIGHT,
    VGA_WIDTH,
};
use crate::x86::{inb, outb};

const VGA_CMD_PORT: u16 = 0x3D4;
const VGA_DATA_PORT: u16 = 0x3D5;

pub fn set_big_cursor() {
    write_cursor_shape(0x00, 0x0F);
}

pub fn set_small_cursor() {
    write_cursor_shape(0x0E, 0x0F);
}

#[allow(dead_code)]
pub fn set_cursor_color(color: u8) {
    write_cursor_shape(color & 0x0F, 0x0F);
}

/*  Couldn't figure this one out. Also, not needed for now
#[allow(dead_code)]
pub fn set_cursor_blinking(blink: bool) {
    out::set_cursor_visible(out::current_screen_index(), blink);
}

#[allow(dead_code)]
pub fn set_cursor_blinking_rate(rate: u8) {
    write_cursor_shape(0x00, rate.min(0x0F));
} */

pub fn set_cursor_shape(start: u8, end: u8) {
    write_cursor_shape(start, end);
}

pub fn set_cursor(x: u16, y: u16) {
    let max_x = (VGA_WIDTH - 1) as u16;
    let max_y = (VGA_HEIGHT - 1) as u16;
    let x = x.min(max_x);
    let y = y.min(max_y);
    let position = (y * VGA_WIDTH as u16 + x) as u16;
    outb(VGA_CMD_PORT, 0x0E);
    outb(VGA_DATA_PORT, (position >> 8) as u8);
    outb(VGA_CMD_PORT, 0x0F);
    outb(VGA_DATA_PORT, (position & 0xFF) as u8);
}

pub fn disable_cursor() {
    outb(VGA_CMD_PORT, 0x0A);
    let cursor_start = inb(VGA_DATA_PORT);
    outb(VGA_DATA_PORT, cursor_start | 0x20);
}

pub fn enable_cursor() {
    outb(VGA_CMD_PORT, 0x0A);
    let cursor_start = inb(VGA_DATA_PORT);
    outb(VGA_DATA_PORT, cursor_start & !0x20);
}
fn write_cursor_shape(start: u8, end: u8) {
    outb(VGA_CMD_PORT, 0x0A);
    let cursor_start = inb(VGA_DATA_PORT);
    outb(VGA_DATA_PORT, (cursor_start & 0xE0) | (start & 0x1F));

    outb(VGA_CMD_PORT, 0x0B);
    let cursor_end = inb(VGA_DATA_PORT);
    outb(VGA_DATA_PORT, (cursor_end & 0xE0) | (end & 0x1F));
}

pub fn move_cursor_on(screen: &mut VirtualScreen, x: usize, y: usize) {
    let clamped_x = x.min(VGA_WIDTH - 1);
    let clamped_y = y.min(SCROLLBACK_LINES - 1);
    screen.cursor.x = clamped_x as u16;
    screen.cursor.y = clamped_y as u16;
    if clamped_y + 1 > screen.used_lines {
        screen.used_lines = clamped_y + 1;
    }

    screen::sync_screen_state(screen);
    screen::render_screen(screen);
}

pub fn move_cursor_left() {
    screen::with_active_screen_mut(|screen| {
        let (cursor, movement) = (screen.cursor, screen.cursor_movement);
        if movement & CursorMovement::Horizontal && cursor.x > 0 {
            move_cursor_on(screen, usize::from(cursor.x - 1), usize::from(cursor.y));
        }
    });
}

pub fn move_cursor_right() {
    screen::with_active_screen_mut(|screen| {
        let (cursor, movement) = (screen.cursor, screen.cursor_movement);
        if movement & CursorMovement::Horizontal && usize::from(cursor.x) + 1 < VGA_WIDTH {
            move_cursor_on(screen, usize::from(cursor.x + 1), usize::from(cursor.y));
        }
    });
}

pub fn move_cursor_up() {
    screen::with_active_screen_mut(|screen| {
        let (cursor, movement) = (screen.cursor, screen.cursor_movement);
        if movement & CursorMovement::Vertical && cursor.y > 0 {
            move_cursor_on(screen, usize::from(cursor.x), usize::from(cursor.y - 1));
        }
    });
}

pub fn move_cursor_down() {
    screen::with_active_screen_mut(|screen| {
        let (cursor, movement) = (screen.cursor, screen.cursor_movement);
        if movement & CursorMovement::Vertical && usize::from(cursor.y) + 1 < SCROLLBACK_LINES {
            move_cursor_on(screen, usize::from(cursor.x), usize::from(cursor.y + 1));
        }
    });
}

pub fn sync_hardware_cursor(screen: &VirtualScreen) {
    if !screen.cursor_visible {
        disable_cursor();
        return;
    }

    let cursor_x = usize::from(screen.cursor.x);
    let cursor_y = usize::from(screen.cursor.y);
    let top_line = screen::visible_top_line_of(screen);

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

pub fn set_cursor_position_on(screen_index: usize, x: u16, y: u16) {
    screen::with_screen_mut(screen_index, |screen| {
        move_cursor_on(screen, x as usize, y as usize)
    });
}
