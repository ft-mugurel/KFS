use super::print;
use super::{
    cursor, Color, ColorCode, CursorMovement, ScreenCursor, ScreenFormatter, VirtualScreen,
};
use super::{
    SCREEN_CONTENT_HEIGHT, SCROLLBACK_LINES, VGA_BUFFER, VGA_HEIGHT, VGA_WIDTH,
    VIRTUAL_SCREENS_COUNT,
};
use crate::pr_err;
use core::cell::UnsafeCell;
use core::fmt::{self, Write};
use core::ops::{BitAnd, BitOr};
use core::sync::atomic::{AtomicUsize, Ordering};

impl Write for ScreenFormatter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        print::write_str_on(self.screen, s);
        Ok(())
    }
}

impl BitAnd for CursorMovement {
    type Output = bool;
    fn bitand(self, rhs: Self) -> bool {
        match (self, rhs) {
            (CursorMovement::Horizontal, CursorMovement::Horizontal)
            | (CursorMovement::Vertical, CursorMovement::Vertical)
            | (CursorMovement::All, CursorMovement::All) => true,
            _ => false,
        }
    }
}

impl BitOr for CursorMovement {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        match (self, rhs) {
            (CursorMovement::Horizontal, CursorMovement::Vertical)
            | (CursorMovement::Vertical, CursorMovement::Horizontal) => CursorMovement::All,
            (CursorMovement::Horizontal, CursorMovement::Horizontal)
            | (CursorMovement::Vertical, CursorMovement::Vertical)
            | (CursorMovement::All, _)
            | (_, CursorMovement::All) => self,
        }
    }
}

impl VirtualScreen {
    pub fn set_color(&mut self, color: ColorCode) {
        self.color = color.clone();
    }

    pub fn set_esc_seq_color(&mut self, color: ColorCode) {
        self.esc_seq_color = Some(color);
    }

    pub fn set_esc_seq_color_background(&mut self, color: Color) {
        let mut color_code = self.esc_seq_color.unwrap_or(self.color.clone());
        color_code.set_background(color);
        self.set_esc_seq_color(color_code);
    }

    pub fn set_esc_seq_color_foreground(&mut self, color: Color) {
        let mut color_code = self.esc_seq_color.unwrap_or(self.color.clone());
        color_code.set_foreground(color);
        self.set_esc_seq_color(color_code);
    }

    pub fn clear_esc_seq_color(&mut self) {
        self.esc_seq_color = None;
    }

    pub fn put_char_at(&mut self, line: usize, column: usize, byte: u8) {
        if line >= SCROLLBACK_LINES || column >= VGA_WIDTH {
            return;
        }

        let color = if let Some(esc_color) = self.esc_seq_color {
            esc_color
        } else {
            self.color
        };
        let vga_char = (byte as u16) | ((color.0 as u16) << 8);
        self.buffer[cell_index(line, column)] = vga_char;
    }

    fn blank_cell(&self) -> u16 {
        (b' ' as u16) | ((self.color.0 as u16) << 8)
    }
}

struct VirtualScreenCell(UnsafeCell<[VirtualScreen; VIRTUAL_SCREENS_COUNT]>);

impl VirtualScreenCell {
    const fn new() -> Self {
        Self(UnsafeCell::new(
            [VirtualScreen {
                index: 0,
                buffer: [0; VGA_WIDTH * SCROLLBACK_LINES],
                cursor: ScreenCursor { x: 0, y: 0 },
                cursor_visible: false,
                used_lines: 1,
                viewport: 0,
                accepts_input: false,
                color: ColorCode::new(Color::LightGray, Color::Black),
                esc_seq_color: None,
                active: false,
                cursor_movement: CursorMovement::All,
            }; VIRTUAL_SCREENS_COUNT],
        ))
    }
}

unsafe impl Sync for VirtualScreenCell {}

static VIRTUAL_SCREENS: VirtualScreenCell = VirtualScreenCell::new();
static ACTIVE_SCREEN_IDX: AtomicUsize = AtomicUsize::new(0);

pub fn with_screen<R>(screen_index: usize, f: impl FnOnce(&VirtualScreen) -> R) -> Option<R> {
    if screen_index >= VIRTUAL_SCREENS_COUNT {
        pr_err!("Invalid screen index: {}\n", screen_index);
        return None;
    }

    let screens = unsafe { &*VIRTUAL_SCREENS.0.get() };
    Some(f(&screens[screen_index]))
}

pub fn with_screen_mut<R>(
    screen_index: usize,
    f: impl FnOnce(&mut VirtualScreen) -> R,
) -> Option<R> {
    if screen_index >= VIRTUAL_SCREENS_COUNT {
        return None;
    }

    let screens = unsafe { &mut *VIRTUAL_SCREENS.0.get() };
    Some(f(&mut screens[screen_index]))
}

pub fn active_screen_index() -> usize {
    ACTIVE_SCREEN_IDX.load(Ordering::Relaxed)
}

pub fn with_active_screen<R>(f: impl FnOnce(&VirtualScreen) -> R) -> Option<R> {
    with_screen(active_screen_index(), f)
}

pub fn with_active_screen_mut<R>(f: impl FnOnce(&mut VirtualScreen) -> R) -> Option<R> {
    let screen_index = active_screen_index();
    with_screen_mut(screen_index, f)
}

fn write_vga_cell(row: usize, column: usize, value: u16) {
    unsafe {
        VGA_BUFFER
            .offset(cell_index(row, column) as isize)
            .write_volatile(value);
    }
}

fn write_bar_cell(column: usize, byte: u8, color: ColorCode) -> usize {
    let value = (byte as u16) | ((color.0 as u16) << 8);
    if column < VGA_WIDTH {
        write_vga_cell(0, column, value);
        column + 1
    } else {
        column
    }
}

fn write_bar_text(mut column: usize, text: &str, color: ColorCode) -> usize {
    for &byte in text.as_bytes() {
        if column >= VGA_WIDTH {
            break;
        }

        column = write_bar_cell(column, byte, color);
    }

    column
}

fn write_bar_digit(column: usize, digit: usize, color: ColorCode) -> usize {
    let digit = match digit {
        0..=9 => b'0' + digit as u8,
        _ => b'?',
    };
    write_bar_cell(column, digit, color)
}

fn render_status_bar(screen_index: usize) {
    let bar_color = ColorCode::new(Color::White, Color::Blue);
    let active_label_color = ColorCode::new(Color::Black, Color::LightGray);

    for column in 0..VGA_WIDTH {
        let _ = write_bar_cell(column, b' ', bar_color);
    }

    let mut column = 0;
    column = write_bar_text(column, "Screen ", bar_color);
    column = write_bar_digit(column, screen_index + 1, bar_color);
    column = write_bar_text(column, " of ", bar_color);
    column = write_bar_digit(column, VIRTUAL_SCREENS_COUNT, bar_color);
    column = write_bar_text(column, " | ", bar_color);

    for index in 0..VIRTUAL_SCREENS_COUNT {
        if index == screen_index {
            column = write_bar_cell(column, b'[', active_label_color);
            column = write_bar_digit(column, index + 1, active_label_color);
            column = write_bar_cell(column, b']', active_label_color);
        } else {
            column = write_bar_digit(column, index + 1, bar_color);
        }

        if index + 1 < VIRTUAL_SCREENS_COUNT && column < VGA_WIDTH {
            column = write_bar_cell(column, b' ', bar_color);
        }
    }

    while column < VGA_WIDTH {
        column = write_bar_cell(column, b' ', bar_color);
    }
}

pub fn cell_index(line: usize, column: usize) -> usize {
    line * VGA_WIDTH + column
}

pub fn clear_buffer_line(screen: &mut VirtualScreen, line: usize) {
    let blank = screen.blank_cell();
    for column in 0..VGA_WIDTH {
        screen.buffer[cell_index(line, column)] = blank;
    }
}

pub fn shift_buffer_up(screen: &mut VirtualScreen) {
    for line in 1..SCROLLBACK_LINES {
        for column in 0..VGA_WIDTH {
            let src = cell_index(line, column);
            let dst = cell_index(line - 1, column);
            screen.buffer[dst] = screen.buffer[src];
        }
    }

    clear_buffer_line(screen, SCROLLBACK_LINES - 1);

    if screen.cursor.y > 0 {
        screen.cursor.y -= 1;
    }
}

pub fn visible_top_line_of(screen: &VirtualScreen) -> usize {
    let used_lines = screen.used_lines.min(SCROLLBACK_LINES);
    let max_scroll = used_lines.saturating_sub(SCREEN_CONTENT_HEIGHT);
    let viewport = screen.viewport.min(max_scroll);
    max_scroll.saturating_sub(viewport)
}

fn render_screen_buffer(screen: &VirtualScreen) {
    render_status_bar(screen.index);

    let top_line = visible_top_line_of(screen);
    let used_lines = screen.used_lines.min(SCROLLBACK_LINES);

    for row in 0..SCREEN_CONTENT_HEIGHT {
        let source_line = top_line + row;
        let target_row = row + 1;
        for column in 0..VGA_WIDTH {
            let value = if source_line < used_lines {
                screen.buffer[cell_index(source_line, column)]
            } else {
                screen.blank_cell()
            };
            write_vga_cell(target_row, column, value);
        }
    }
}

pub fn render_screen(screen: &mut VirtualScreen) {
    render_screen_buffer(screen);
    sync_cursor_of(screen);
}

pub fn render_cell_if_visible_of(screen: &VirtualScreen, line: usize, column: usize) {
    if line >= SCROLLBACK_LINES || column >= VGA_WIDTH {
        return;
    }

    let top_line = visible_top_line_of(screen);
    if line < top_line || line >= top_line + VGA_HEIGHT {
        return;
    }

    let used_lines = screen.used_lines.min(SCROLLBACK_LINES);
    let value = if line < used_lines {
        screen.buffer[cell_index(line, column)]
    } else {
        screen.blank_cell()
    };

    let row = line - top_line + 1;
    write_vga_cell(row, column, value);
}

pub fn sync_cursor_of(screen: &VirtualScreen) {
    cursor::sync_hardware_cursor(screen);
}

pub fn sync_screen_state(screen: &mut VirtualScreen) {
    screen.used_lines = screen.used_lines.min(SCROLLBACK_LINES);
    screen.viewport = screen
        .viewport
        .min(screen.used_lines.saturating_sub(SCREEN_CONTENT_HEIGHT));
}

pub fn set_active(screen_index: usize) {
    with_active_screen_mut(|screen| {
        screen.active = false;
    });
    with_screen_mut(screen_index, |screen| {
        screen.active = true;
        render_screen(screen);
    });
    ACTIVE_SCREEN_IDX.store(screen_index, Ordering::Relaxed);
}

pub fn init() {
    for screen_index in 0..VIRTUAL_SCREENS_COUNT {
        with_screen_mut(screen_index, |screen| {
            screen.index = screen_index;
            for line in 0..SCROLLBACK_LINES {
                clear_buffer_line(screen, line);
            }
            screen.accepts_input = false;
        });
    }

    set_active(0);
}

pub fn scroll_view_to_top() {
    with_active_screen_mut(|screen| {
        screen.viewport = screen.used_lines.saturating_sub(SCREEN_CONTENT_HEIGHT);
        render_screen(screen);
    });
}

pub fn scroll_view_to_bottom() {
    with_active_screen_mut(|screen| {
        screen.viewport = 0;
        render_screen(screen);
    });
}

pub fn scroll_view_up() {
    with_active_screen_mut(|screen| {
        let max_scroll = screen.used_lines.saturating_sub(SCREEN_CONTENT_HEIGHT);
        if screen.viewport < max_scroll {
            screen.viewport += 1;
        }
        render_screen(screen);
    });
}

pub fn scroll_view_down() {
    with_active_screen_mut(|screen| {
        if screen.viewport > 0 {
            screen.viewport -= 1;
        }
        render_screen(screen);
    });
}

pub fn is_screen_active(screen_index: usize) -> bool {
    active_screen_index() == screen_index
}

pub fn active_screen_accepts_input() -> bool {
    if let Some(accepts_input) = with_active_screen(|screen| screen.accepts_input) {
        accepts_input
    } else {
        false
    }
}

pub fn active_cursor_position() -> (u16, u16) {
    if let Some((x, y)) = with_active_screen(|screen| (screen.cursor.x, screen.cursor.y)) {
        (x, y)
    } else {
        (0, 1)
    }
}

pub fn set_screen_accepts_input(screen_index: usize, accepts_input: bool) {
    with_screen_mut(screen_index, |screen| {
        screen.accepts_input = accepts_input;
        if screen.cursor_visible != screen.accepts_input {
            screen.cursor_visible = screen.accepts_input;
            render_screen(screen);
        }
    });
}

pub fn switch_to_previous_screen() {
    let current_index = active_screen_index();
    let previous_index = (current_index + VIRTUAL_SCREENS_COUNT - 1) % VIRTUAL_SCREENS_COUNT;
    set_active(previous_index);
}

pub fn switch_to_next_screen() {
    let current_index = active_screen_index();
    let next_index = (current_index + 1) % VIRTUAL_SCREENS_COUNT;
    set_active(next_index);
}

pub fn set_cursor_movement_on(screen_index: usize, mode: CursorMovement) {
    with_screen_mut(screen_index, |screen| {
        screen.cursor_movement = mode;
    });
}

pub fn change_color(color: ColorCode) {
    with_active_screen_mut(|screen| {
        screen.set_color(color);
    });
}

pub fn clear(screen_index: usize) {
    with_screen_mut(screen_index, |screen| {
        for line in 0..SCROLLBACK_LINES {
            clear_buffer_line(screen, line);
        }
        screen.cursor = ScreenCursor { x: 0, y: 0 };
        screen.used_lines = 1;
        screen.viewport = 0;
        render_screen(screen);
    });
}
