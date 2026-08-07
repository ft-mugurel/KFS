use crate::startup_config;

mod cursor;
mod print;
mod screen;

pub(self) const VIRTUAL_SCREENS_COUNT: usize = startup_config::vga::VIRTUAL_SCREENS;
pub(self) const SCREEN_CONTENT_HEIGHT: usize = startup_config::vga::CONTENT_HEIGHT;
pub(self) const SCROLLBACK_LINES: usize = startup_config::vga::SCROLLBACK_LINES;
pub(self) const VGA_HEIGHT: usize = startup_config::vga::HEIGHT;
pub(self) const VGA_BUFFER: *mut u16 = startup_config::vga::BUFFER_ADDR as *mut u16;
pub(crate) const VGA_WIDTH: usize = startup_config::vga::WIDTH;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub(crate) struct ColorCode(pub u8);

#[derive(Copy, Clone)]
pub(self) struct ScreenCursor {
    pub x: u16,
    pub y: u16,
}

#[derive(Clone, Copy)]
pub(self) struct VirtualScreen {
    pub index: usize,
    pub buffer: [u16; VGA_WIDTH * SCROLLBACK_LINES],
    pub cursor: ScreenCursor,
    pub cursor_visible: bool,
    pub used_lines: usize,
    pub viewport: usize,
    pub accepts_input: bool,
    pub color: ColorCode,
    pub esc_seq_color: Option<ColorCode>,
    pub active: bool,
    pub cursor_movement: CursorMovement,
}

pub(self) struct ScreenFormatter<'a> {
    pub screen: &'a mut VirtualScreen,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CursorMovement {
    Horizontal = 0x01,
    Vertical = 0x02,
    All = 0x03,
}

pub(crate) use cursor::{
    disable_cursor, enable_cursor, move_cursor_down, move_cursor_left, move_cursor_right,
    move_cursor_up, set_big_cursor, set_cursor_position_on, set_cursor_shape, set_small_cursor,
};
pub(crate) use print::{print_char_on, print_fmt_on, print_str_on};
pub(crate) use screen::{
    active_cursor_position, active_screen_accepts_input, active_screen_index, change_color, clear,
    init_virtual_screens, is_screen_active, scroll_view_down, scroll_view_to_bottom,
    scroll_view_to_top, scroll_view_up, set_active as switch_screen, set_cursor_movement_on,
    set_screen_accepts_input, switch_to_next_screen, switch_to_previous_screen,
};
