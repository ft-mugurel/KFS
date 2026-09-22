use core::str;
use crate::interrupts::keyboard::{self, KeyboardLayout};
use crate::printk::set_log_level;
use crate::shell::init::print;
use crate::vga::text_mod::{change_color, switch_screen, Color, ColorCode};
use super::parse::{parse_color, parse_log_level};

#[inline(always)]
pub(crate) fn command_screen(mut parts: str::SplitWhitespace<'_>) {
    let Some(arg) = parts.next() else {
        print("usage: screen <1-6>\n");
        return;
    };

    let Ok(screen) = arg.parse::<usize>() else {
        print("invalid screen index\n");
        return;
    };

    if !(1..=6).contains(&screen) {
        print("screen index must be in range 1-6\n");
        return;
    }

    switch_screen(screen - 1);
}

#[inline(always)]
pub(crate) fn command_loglevel(mut parts: str::SplitWhitespace<'_>) {
    let Some(arg) = parts.next() else {
        print("usage: loglevel <emerg|alert|crit|err|warn|notice|info|debug>\n");
        return;
    };

    let Some(level) = parse_log_level(arg) else {
        print("invalid log level\n");
        return;
    };

    set_log_level(level);
    print("log level updated\n");
}

#[inline(always)]
pub(crate) fn command_color(mut parts: str::SplitWhitespace<'_>) {
    let Some(arg) = parts.next() else {
        print("usage: color <white|gray|red|green|blue|yellow|cyan|magenta>\n");
        return;
    };

    let Some(color) = parse_color(arg) else {
        print("invalid color\n");
        return;
    };

    change_color(ColorCode::new(color, Color::Black));
    print("shell color updated\n");
}

#[inline(always)]
pub(crate) fn command_layout(mut parts: str::SplitWhitespace<'_>) {
    let Some(layout_str) = parts.next() else {
        print("usage: layout <us|tr>\n");
        return;
    };

    let layout = match layout_str {
        "us" => KeyboardLayout::UsQwerty,
        "tr" => KeyboardLayout::TrQwerty,
        _ => {
            print("invalid layout\n");
            return;
        }
    };

    keyboard::set_layout(layout);
    print("keyboard layout updated\n");
}
