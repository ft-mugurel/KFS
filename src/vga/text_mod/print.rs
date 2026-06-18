use crate::vga::text_mod::out::Color;

use super::screen;

#[derive(Copy, Clone)]
struct WriteOutcome {
    changed_cell: Option<(usize, usize)>,
    force_full_redraw: bool,
}

fn finalize_write(
    screen: &mut screen::VirtualScreen,
    top_line_before: usize,
    outcome: WriteOutcome,
) {
    if !screen::is_screen_active(screen) {
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

fn newline_with_scroll(screen: &mut screen::VirtualScreen) -> bool {
    screen.cursor.x = 0;
    let mut force_full_redraw = false;

    let next_line = usize::from(screen.cursor.y) + 1;
    if next_line >= screen::SCROLLBACK_LINES {
        screen::shift_buffer_up(screen);
        screen.cursor.y = (screen::SCROLLBACK_LINES - 1) as u16;
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

fn backspace(screen: &mut screen::VirtualScreen) -> Option<(usize, usize)> {
    if screen.cursor.x > 0 {
        screen.cursor.x -= 1;
    } else if screen.cursor.y > 0 {
        screen.cursor.y -= 1;
        screen.cursor.x = (screen::VGA_WIDTH - 1) as u16;
    } else {
        return None;
    }

    let line = usize::from(screen.cursor.y);
    let column = usize::from(screen.cursor.x);
    screen.put_char_at(line, column, b' ');

    Some((line, column))
}

fn write_raw_byte(screen: &mut screen::VirtualScreen, byte: u8) -> WriteOutcome {
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
            if next_column >= screen::VGA_WIDTH {
                next_column = screen::VGA_WIDTH - 1;
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
            if next_x >= screen::VGA_WIDTH {
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

pub(super) fn write_str_on(screen: &mut screen::VirtualScreen, text: &str) {
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

pub(super) fn write_char_on(screen: &mut screen::VirtualScreen, c: char) {
    let byte = if (c as u32) <= 0xFF { c as u8 } else { b'?' };
    let top_line_before = screen::visible_top_line_of(screen);
    let outcome = write_raw_byte(screen, byte);
    finalize_write(screen, top_line_before, outcome);
}
