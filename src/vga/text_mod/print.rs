use super::{
    screen, Color, ColorCode, ScreenFormatter, VirtualScreen, SCROLLBACK_LINES, VGA_WIDTH,
};
use core::fmt::{self, Arguments, Display, Write};

#[derive(Copy, Clone)]
struct WriteOutcome {
    changed_cell: Option<(usize, usize)>,
    force_full_redraw: bool,
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

fn finalize_write(screen: &mut VirtualScreen, top_line_before: usize, outcome: WriteOutcome) {
    if !screen.active {
        return;
    }

    screen::sync_screen_state(screen);

    let top_line_after = screen::visible_top_line_of(screen);
    if outcome.force_full_redraw || top_line_after != top_line_before {
        screen::render_screen(screen);
        return;
    }

    if let Some((line, column)) = outcome.changed_cell {
        screen::render_cell_if_visible_of(screen, line, column);
    }

    screen::sync_cursor_of(screen);
}

fn newline_with_scroll(screen: &mut VirtualScreen) -> bool {
    screen.cursor.x = 0;
    let mut force_full_redraw = false;

    let next_line = usize::from(screen.cursor.y) + 1;
    if next_line >= SCROLLBACK_LINES {
        screen::shift_buffer_up(screen);
        screen.cursor.y = (SCROLLBACK_LINES - 1) as u16;
        force_full_redraw = true;
    } else {
        screen.cursor.y = next_line as u16;
    }

    let cursor_line = usize::from(screen.cursor.y);
    if cursor_line + 1 > screen.used_lines {
        screen.used_lines = cursor_line + 1;
        screen::clear_buffer_line(screen, cursor_line);
    }

    force_full_redraw
}

fn backspace(screen: &mut VirtualScreen) -> Option<(usize, usize)> {
    if screen.cursor.x > 0 {
        screen.cursor.x -= 1;
    } else if screen.cursor.y > 0 {
        screen.cursor.y -= 1;
        screen.cursor.x = (VGA_WIDTH - 1) as u16;
    } else {
        return None;
    }

    let line = usize::from(screen.cursor.y);
    let column = usize::from(screen.cursor.x);
    screen.put_char_at(line, column, b' ');

    Some((line, column))
}

fn write_raw_byte(screen: &mut VirtualScreen, byte: u8) -> WriteOutcome {
    if byte == b'\n' {
        for _ in 0..1000 {
            if (crate::x86::inb(0x3F8 + 5) & 0x20) != 0 {
                break;
            }
        }
        crate::x86::outb(0x3F8, b'\r');
    }
    for _ in 0..1000 {
        if (crate::x86::inb(0x3F8 + 5) & 0x20) != 0 {
            break;
        }
    }
    crate::x86::outb(0x3F8, byte);
    match byte {
        b'\n' => {
            let force_full_redraw = newline_with_scroll(screen);
            WriteOutcome { changed_cell: None, force_full_redraw }
        }
        b'\r' => {
            screen.cursor.x = 0;
            WriteOutcome { changed_cell: None, force_full_redraw: false }
        }
        0x08 => {
            let changed_cell = backspace(screen);
            WriteOutcome { changed_cell, force_full_redraw: false }
        }
        b'\t' => {
            let line = usize::from(screen.cursor.y);
            let column = usize::from(screen.cursor.x);
            let mut next_column = column + 4 - (column % 4);
            if next_column >= VGA_WIDTH {
                next_column = VGA_WIDTH - 1;
            }
            screen.cursor.x = next_column as u16;
            screen.cursor.y = line as u16;
            WriteOutcome { changed_cell: None, force_full_redraw: false }
        }
        byte => {
            let line = usize::from(screen.cursor.y);
            let column = usize::from(screen.cursor.x);
            screen.put_char_at(line, column, byte);

            let next_x = column + 1;
            if next_x >= VGA_WIDTH {
                let force_full_redraw = newline_with_scroll(screen);
                WriteOutcome { changed_cell: Some((line, column)), force_full_redraw }
            } else {
                screen.cursor.x = next_x as u16;
                screen.cursor.y = line as u16;
                WriteOutcome {
                    changed_cell: Some((line, column)),
                    force_full_redraw: false,
                }
            }
        }
    }
}

pub fn write_str_on(screen: &mut VirtualScreen, text: &str) {
    let mut escape_mode = false;
    for &byte in text.as_bytes() {
        if byte == 0x1B {
            escape_mode = true;
            screen.clear_esc_seq_color();
            continue;
        }

        if !escape_mode {
            let top_line_before = screen::visible_top_line_of(screen);
            let outcome = write_raw_byte(screen, byte);
            finalize_write(screen, top_line_before, outcome);
        } else {
            /*
             * 0x10      -> is_background flag
             * 0x00-0x0F -> colors
             * ';'       -> separator for multiple color codes
             * 'm'       -> end of escape sequence
             *
             * Empty sequences or unrecognized codes will reset the modifications
             * Refer to vga::text_mod::out::Color for mapping
             */
            if byte <= 0x20 {
                let is_background = byte & 0x10 != 0;
                let color_code = byte & 0x0F;
                if is_background {
                    screen.set_esc_seq_color_background(Color::from_u8(color_code));
                } else {
                    screen.set_esc_seq_color_foreground(Color::from_u8(color_code));
                }
            } else if byte == b';' {
                continue;
            } else if byte == b'm' {
                escape_mode = false;
            } else {
                screen.clear_esc_seq_color();
                escape_mode = false;
            }
        }
    }
}

pub fn print_char_on(screen_index: usize, c: char) {
    screen::with_screen_mut(screen_index, |screen| {
        let byte = if (c as u32) <= 0xFF { c as u8 } else { b'?' };
        let top_line_before = screen::visible_top_line_of(screen);
        let outcome = write_raw_byte(screen, byte);
        finalize_write(screen, top_line_before, outcome);
    });
}

pub fn print_str_on(screen_index: usize, str: &str) {
    screen::with_screen_mut(screen_index, |screen| {
        super::print::write_str_on(screen, str);
    });
}

pub fn print_fmt_on(screen_index: usize, args: &Arguments<'_>) {
    screen::with_screen_mut(screen_index, |screen| {
        let mut formatter = ScreenFormatter { screen };
        let _ = formatter.write_fmt(*args);
    });
}
