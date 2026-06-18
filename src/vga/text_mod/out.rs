use super::cursor;
use super::screen;
use core::fmt;
use core::fmt::Display;
use core::fmt::Write;

struct ScreenFormatter<'a> {
    screen: &'a mut screen::VirtualScreen,
}

impl fmt::Write for ScreenFormatter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        super::print::write_str_on(self.screen, s);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
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

impl Color {
    pub const fn from_u8(code: u8) -> Color {
        match code {
            0 => Color::Black,
            1 => Color::Blue,
            2 => Color::Green,
            3 => Color::Cyan,
            4 => Color::Red,
            5 => Color::Magenta,
            6 => Color::Brown,
            7 => Color::LightGray,
            8 => Color::DarkGray,
            9 => Color::LightBlue,
            10 => Color::LightGreen,
            11 => Color::LightCyan,
            12 => Color::LightRed,
            13 => Color::Pink,
            14 => Color::Yellow,
            _ => Color::White,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct ColorCode(pub u8);

impl ColorCode {
    pub const fn new(foreground: Color, background: Color) -> ColorCode {
        ColorCode((background as u8) << 4 | (foreground as u8))
    }
    #[inline(always)]
    pub fn foreground(&self) -> Color {
        Color::from_u8(self.0 & 0x0F)
    }
    #[inline(always)]
    pub fn background(&self) -> Color {
        Color::from_u8((self.0 >> 4) & 0x0F)
    }
    #[inline(always)]
    pub fn set_foreground(&mut self, foreground: Color) {
        self.0 = (self.0 & 0xF0) | (foreground as u8 & 0x0F);
    }
    #[inline(always)]
    pub fn set_background(&mut self, background: Color) {
        self.0 = (self.0 & 0x0F) | ((background as u8 & 0x0F) << 4);
    }
}

impl Display for ColorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ColorCode(fg: {:?}, bg: {:?})",
            self.foreground(),
            self.background()
        )
    }
}

pub fn active_screen_accepts_input() -> bool {
    if let Some(accepts_input) = screen::with_active_screen(|screen| screen.accepts_input) {
        accepts_input
    } else {
        false
    }
}

pub fn set_screen_accepts_input(screen_index: usize, accepts_input: bool) {
    screen::with_screen_mut(screen_index, |screen| {
        screen.accepts_input = accepts_input;
        if screen.cursor_visible != screen.accepts_input {
            screen.cursor_visible = screen.accepts_input;
            screen::render_screen(screen);
        }
    });
}

pub fn active_cursor_position() -> (u16, u16) {
    if let Some((x, y)) = screen::with_active_screen(|screen| (screen.cursor.x, screen.cursor.y)) {
        (x, y)
    } else {
        (0, 1)
    }
}

pub fn set_cursor_position_on(screen_index: usize, x: u16, y: u16) {
    screen::with_screen_mut(screen_index, |screen| {
        cursor::move_on(screen, x as usize, y as usize)
    });
}

pub fn change_color(color: ColorCode) {
    screen::with_active_screen_mut(|screen| {
        screen.set_color(color);
    });
}

pub fn clear(screen_index: usize) {
    screen::with_screen_mut(screen_index, |screen| {
        for line in 0..screen::SCROLLBACK_LINES {
            screen::clear_buffer_line(screen, line);
        }
        screen.cursor = cursor::ScreenCursor { x: 0, y: 0 };
        screen.used_lines = 1;
        screen.viewport = 0;
        screen::render_screen(screen);
    });
}

pub fn print_on(screen_index: usize, str: &str) {
    screen::with_screen_mut(screen_index, |screen| {
        super::print::write_str_on(screen, str);
    });
}

pub fn write_fmt_on(screen_index: usize, args: &fmt::Arguments<'_>) {
    screen::with_screen_mut(screen_index, |screen| {
        let mut formatter = ScreenFormatter { screen };
        let _ = formatter.write_fmt(*args);
    });
}

pub fn print_char_on(screen_index: usize, c: char) {
    screen::with_screen_mut(screen_index, |screen| {
        super::print::write_char_on(screen, c);
    });
}

pub fn move_cursor_left() {
    cursor::move_left();
}

pub fn move_cursor_right() {
    cursor::move_right();
}

pub fn move_cursor_up() {
    cursor::move_up();
}

pub fn move_cursor_down() {
    cursor::move_down();
}

pub fn scroll_view_to_top() {
    screen::with_active_screen_mut(|screen| {
        screen.viewport = screen
            .used_lines
            .saturating_sub(screen::SCREEN_CONTENT_HEIGHT);
        screen::render_screen(screen);
    });
}

pub fn scroll_view_to_bottom() {
    screen::with_active_screen_mut(|screen| {
        screen.viewport = 0;
        screen::render_screen(screen);
    });
}

pub fn scroll_view_up() {
    screen::with_active_screen_mut(|screen| {
        let max_scroll = screen
            .used_lines
            .saturating_sub(screen::SCREEN_CONTENT_HEIGHT);
        if screen.viewport < max_scroll {
            screen.viewport += 1;
        }
        screen::render_screen(screen);
    });
}

pub fn scroll_view_down() {
    screen::with_active_screen_mut(|screen| {
        if screen.viewport > 0 {
            screen.viewport -= 1;
        }
        screen::render_screen(screen);
    });
}

pub fn init_virtual_screens() {
    screen::init();
}

pub fn switch_screen(screen_index: usize) {
    screen::set_active(screen_index);
}

pub fn is_screen_active(screen_index: usize) -> bool {
    screen::active_screen_index() == screen_index
}

pub fn set_cursor_movement_on(screen_index: usize, mode: screen::CursorMovement) {
    screen::with_screen_mut(screen_index, |screen| {
        screen.cursor_movement = mode;
    });
}

pub fn switch_to_previous_screen() {
    let current_index = screen::active_screen_index();
    let previous_index =
        (current_index + screen::VIRTUAL_SCREENS_COUNT - 1) % screen::VIRTUAL_SCREENS_COUNT;
    switch_screen(previous_index);
}

pub fn switch_to_next_screen() {
    let current_index = screen::active_screen_index();
    let next_index = (current_index + 1) % screen::VIRTUAL_SCREENS_COUNT;
    switch_screen(next_index);
}
