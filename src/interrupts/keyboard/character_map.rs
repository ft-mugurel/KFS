use core::sync::atomic::{AtomicU8, Ordering};

use crate::interrupts::keyboard::keycode::{KeyCode, Modifiers};

#[derive(Clone, Copy)]
pub struct Glyph {
    base: char,
    lvl2: char,
    lvl3: Option<char>,
    lvl5: Option<char>,
    is_letter: bool,
}

impl Glyph {
    const fn new_l2(base: char, lvl2: char, is_letter: bool) -> Self {
        Self { base, lvl2, lvl3: None, lvl5: None, is_letter }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum KeyboardLayout {
    UsQwerty = 0,
    TrQwerty = 1,
    GbQwerty = 2,

    LayoutEnd = 3,
}

static ACTIVE_LAYOUT: AtomicU8 = AtomicU8::new(KeyboardLayout::UsQwerty as u8);

pub fn set_layout(layout: KeyboardLayout) {
    ACTIVE_LAYOUT.store(layout as u8, Ordering::Relaxed);
}

pub fn get_layout() -> KeyboardLayout {
    match ACTIVE_LAYOUT.load(Ordering::Relaxed) {
        0 => KeyboardLayout::UsQwerty,
        1 => KeyboardLayout::TrQwerty,
        2 => KeyboardLayout::GbQwerty,
        _ => unreachable!(),
    }
}

pub fn toggle_layout() {
    ACTIVE_LAYOUT
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
            let next = (current + 1) % (KeyboardLayout::LayoutEnd as u8);
            Some(next)
        })
        .ok();
}

pub fn keycode_to_char(key: KeyCode, modifiers: Modifiers) -> Option<char> {
    let layout = get_layout();
    let glyph = match layout {
        KeyboardLayout::UsQwerty => us_qwerty::glyph_for_key(key),
        KeyboardLayout::TrQwerty => tr_qwerty::glyph_for_key(key),
        KeyboardLayout::GbQwerty => gb_qwerty::glyph_for_key(key),
        _ => None,
    }?;

    if modifiers.shift() ^ (modifiers.caps_lock() && glyph.is_letter) {
        Some(glyph.lvl2)
    } else {
        Some(glyph.base)
    }
}

mod us_qwerty {
    use crate::interrupts::keyboard::character_map::Glyph;
    use crate::interrupts::keyboard::keycode::KeyCode;

    pub(super) fn glyph_for_key(key: KeyCode) -> Option<Glyph> {
        let glyph = match key {
            KeyCode::Backspace => Glyph::new_l2('\x08', '\x08', false),
            KeyCode::Tab => Glyph::new_l2('\t', '\t', false),
            KeyCode::Enter => Glyph::new_l2('\n', '\n', false),
            KeyCode::Space => Glyph::new_l2(' ', ' ', false),
            KeyCode::Digit1 => Glyph::new_l2('1', '!', false),
            KeyCode::Digit2 => Glyph::new_l2('2', '@', false),
            KeyCode::Digit3 => Glyph::new_l2('3', '#', false),
            KeyCode::Digit4 => Glyph::new_l2('4', '$', false),
            KeyCode::Digit5 => Glyph::new_l2('5', '%', false),
            KeyCode::Digit6 => Glyph::new_l2('6', '^', false),
            KeyCode::Digit7 => Glyph::new_l2('7', '&', false),
            KeyCode::Digit8 => Glyph::new_l2('8', '*', false),
            KeyCode::Digit9 => Glyph::new_l2('9', '(', false),
            KeyCode::Digit0 => Glyph::new_l2('0', ')', false),
            KeyCode::Minus => Glyph::new_l2('-', '_', false),
            KeyCode::Equal => Glyph::new_l2('=', '+', false),
            KeyCode::LeftBracket => Glyph::new_l2('[', '{', false),
            KeyCode::RightBracket => Glyph::new_l2(']', '}', false),
            KeyCode::Backslash => Glyph::new_l2('\\', '|', false),
            KeyCode::Semicolon => Glyph::new_l2(';', ':', false),
            KeyCode::Apostrophe => Glyph::new_l2('\'', '"', false),
            KeyCode::Grave => Glyph::new_l2('`', '~', false),
            KeyCode::Comma => Glyph::new_l2(',', '<', false),
            KeyCode::Dot => Glyph::new_l2('.', '>', false),
            KeyCode::Slash => Glyph::new_l2('/', '?', false),
            KeyCode::A => Glyph::new_l2('a', 'A', true),
            KeyCode::B => Glyph::new_l2('b', 'B', true),
            KeyCode::C => Glyph::new_l2('c', 'C', true),
            KeyCode::D => Glyph::new_l2('d', 'D', true),
            KeyCode::E => Glyph::new_l2('e', 'E', true),
            KeyCode::F => Glyph::new_l2('f', 'F', true),
            KeyCode::G => Glyph::new_l2('g', 'G', true),
            KeyCode::H => Glyph::new_l2('h', 'H', true),
            KeyCode::I => Glyph::new_l2('i', 'I', true),
            KeyCode::J => Glyph::new_l2('j', 'J', true),
            KeyCode::K => Glyph::new_l2('k', 'K', true),
            KeyCode::L => Glyph::new_l2('l', 'L', true),
            KeyCode::M => Glyph::new_l2('m', 'M', true),
            KeyCode::N => Glyph::new_l2('n', 'N', true),
            KeyCode::O => Glyph::new_l2('o', 'O', true),
            KeyCode::P => Glyph::new_l2('p', 'P', true),
            KeyCode::Q => Glyph::new_l2('q', 'Q', true),
            KeyCode::R => Glyph::new_l2('r', 'R', true),
            KeyCode::S => Glyph::new_l2('s', 'S', true),
            KeyCode::T => Glyph::new_l2('t', 'T', true),
            KeyCode::U => Glyph::new_l2('u', 'U', true),
            KeyCode::V => Glyph::new_l2('v', 'V', true),
            KeyCode::W => Glyph::new_l2('w', 'W', true),
            KeyCode::X => Glyph::new_l2('x', 'X', true),
            KeyCode::Y => Glyph::new_l2('y', 'Y', true),
            KeyCode::Z => Glyph::new_l2('z', 'Z', true),
            _ => return None,
        };

        Some(glyph)
    }
}

mod tr_qwerty {
    use crate::interrupts::keyboard::character_map::Glyph;
    use crate::interrupts::keyboard::keycode::KeyCode;

    pub(super) fn glyph_for_key(key: KeyCode) -> Option<Glyph> {
        let glyph = match key {
            KeyCode::Backspace => Glyph::new_l2('\x08', '\x08', false),
            KeyCode::Tab => Glyph::new_l2('\t', '\t', false),
            KeyCode::Enter => Glyph::new_l2('\n', '\n', false),
            KeyCode::Space => Glyph::new_l2(' ', ' ', false),
            KeyCode::Digit1 => Glyph::new_l2('1', '!', false),
            KeyCode::Digit2 => Glyph::new_l2('2', '\'', false),
            KeyCode::Digit3 => Glyph::new_l2('3', '^', false),
            KeyCode::Digit4 => Glyph::new_l2('4', '+', false),
            KeyCode::Digit5 => Glyph::new_l2('5', '%', false),
            KeyCode::Digit6 => Glyph::new_l2('6', '&', false),
            KeyCode::Digit7 => Glyph::new_l2('7', '/', false),
            KeyCode::Digit8 => Glyph::new_l2('8', '(', false),
            KeyCode::Digit9 => Glyph::new_l2('9', ')', false),
            KeyCode::Digit0 => Glyph::new_l2('0', '=', false),
            KeyCode::Minus => Glyph::new_l2('*', '?', false),
            KeyCode::Equal => Glyph::new_l2('-', '_', false),
            KeyCode::LeftBracket => Glyph::new_l2('ğ', 'Ğ', true),
            KeyCode::RightBracket => Glyph::new_l2('ü', 'Ü', true),
            KeyCode::Backslash => Glyph::new_l2('<', '>', false),
            KeyCode::Semicolon => Glyph::new_l2('ş', 'Ş', true),
            KeyCode::Apostrophe => Glyph::new_l2('i', 'İ', true),
            KeyCode::Grave => Glyph::new_l2('"', 'é', false),
            KeyCode::Comma => Glyph::new_l2('ö', 'Ö', false),
            KeyCode::Dot => Glyph::new_l2('ç', 'Ç', false),
            KeyCode::Slash => Glyph::new_l2('.', ':', false),
            KeyCode::A => Glyph::new_l2('a', 'A', true),
            KeyCode::B => Glyph::new_l2('b', 'B', true),
            KeyCode::C => Glyph::new_l2('c', 'C', true),
            KeyCode::D => Glyph::new_l2('d', 'D', true),
            KeyCode::E => Glyph::new_l2('e', 'E', true),
            KeyCode::F => Glyph::new_l2('f', 'F', true),
            KeyCode::G => Glyph::new_l2('g', 'G', true),
            KeyCode::H => Glyph::new_l2('h', 'H', true),
            KeyCode::I => Glyph::new_l2('i', 'I', true),
            KeyCode::J => Glyph::new_l2('j', 'J', true),
            KeyCode::K => Glyph::new_l2('k', 'K', true),
            KeyCode::L => Glyph::new_l2('l', 'L', true),
            KeyCode::M => Glyph::new_l2('m', 'M', true),
            KeyCode::N => Glyph::new_l2('n', 'N', true),
            KeyCode::O => Glyph::new_l2('o', 'O', true),
            KeyCode::P => Glyph::new_l2('p', 'P', true),
            KeyCode::Q => Glyph::new_l2('q', 'Q', true),
            KeyCode::R => Glyph::new_l2('r', 'R', true),
            KeyCode::S => Glyph::new_l2('s', 'S', true),
            KeyCode::T => Glyph::new_l2('t', 'T', true),
            KeyCode::U => Glyph::new_l2('u', 'U', true),
            KeyCode::V => Glyph::new_l2('v', 'V', true),
            KeyCode::W => Glyph::new_l2('w', 'W', true),
            KeyCode::X => Glyph::new_l2('x', 'X', true),
            KeyCode::Y => Glyph::new_l2('y', 'Y', true),
            KeyCode::Z => Glyph::new_l2('z', 'Z', true),
            _ => return None,
        };

        Some(glyph)
    }
}

mod gb_qwerty {
    use crate::interrupts::keyboard::character_map::Glyph;
    use crate::interrupts::keyboard::keycode::KeyCode;

    pub(super) fn glyph_for_key(key: KeyCode) -> Option<Glyph> {
        let glyph = match key {
            KeyCode::Backspace => Glyph::new_l2('\x08', '\x08', false),
            KeyCode::Tab => Glyph::new_l2('\t', '\t', false),
            KeyCode::Enter => Glyph::new_l2('\n', '\n', false),
            KeyCode::Space => Glyph::new_l2(' ', ' ', false),
            KeyCode::Digit1 => Glyph::new_l2('1', '!', false),
            KeyCode::Digit2 => Glyph::new_l2('2', '"', false),
            KeyCode::Digit3 => Glyph::new_l2('3', '£', false),
            KeyCode::Digit4 => Glyph::new_l2('4', '$', false),
            KeyCode::Digit5 => Glyph::new_l2('5', '%', false),
            KeyCode::Digit6 => Glyph::new_l2('6', '^', false),
            KeyCode::Digit7 => Glyph::new_l2('7', '&', false),
            KeyCode::Digit8 => Glyph::new_l2('8', '*', false),
            KeyCode::Digit9 => Glyph::new_l2('9', '(', false),
            KeyCode::Digit0 => Glyph::new_l2('0', ')', false),
            KeyCode::Minus => Glyph::new_l2('-', '_', false),
            KeyCode::Equal => Glyph::new_l2('=', '+', false),
            KeyCode::LeftBracket => Glyph::new_l2('[', '{', false),
            KeyCode::RightBracket => Glyph::new_l2(']', '}', false),
            KeyCode::Backslash => Glyph::new_l2('\\', '|', false),
            KeyCode::Semicolon => Glyph::new_l2(';', ':', false),
            KeyCode::Apostrophe => Glyph::new_l2('\'', '@', false),
            KeyCode::Grave => Glyph::new_l2('`', '~', false),
            KeyCode::Comma => Glyph::new_l2(',', '<', false),
            KeyCode::Dot => Glyph::new_l2('.', '>', false),
            KeyCode::Slash => Glyph::new_l2('/', '?', false),
            KeyCode::A => Glyph::new_l2('a', 'A', true),
            KeyCode::B => Glyph::new_l2('b', 'B', true),
            KeyCode::C => Glyph::new_l2('c', 'C', true),
            KeyCode::D => Glyph::new_l2('d', 'D', true),
            KeyCode::E => Glyph::new_l2('e', 'E', true),
            KeyCode::F => Glyph::new_l2('f', 'F', true),
            KeyCode::G => Glyph::new_l2('g', 'G', true),
            KeyCode::H => Glyph::new_l2('h', 'H', true),
            KeyCode::I => Glyph::new_l2('i', 'I', true),
            KeyCode::J => Glyph::new_l2('j', 'J', true),
            KeyCode::K => Glyph::new_l2('k', 'K', true),
            KeyCode::L => Glyph::new_l2('l', 'L', true),
            KeyCode::M => Glyph::new_l2('m', 'M', true),
            KeyCode::N => Glyph::new_l2('n', 'N', true),
            KeyCode::O => Glyph::new_l2('o', 'O', true),
            KeyCode::P => Glyph::new_l2('p', 'P', true),
            KeyCode::Q => Glyph::new_l2('q', 'Q', true),
            KeyCode::R => Glyph::new_l2('r', 'R', true),
            KeyCode::S => Glyph::new_l2('s', 'S', true),
            KeyCode::T => Glyph::new_l2('t', 'T', true),
            KeyCode::U => Glyph::new_l2('u', 'U', true),
            KeyCode::V => Glyph::new_l2('v', 'V', true),
            KeyCode::W => Glyph::new_l2('w', 'W', true),
            KeyCode::X => Glyph::new_l2('x', 'X', true),
            KeyCode::Y => Glyph::new_l2('y', 'Y', true),
            KeyCode::Z => Glyph::new_l2('z', 'Z', true),
            _ => return None,
        };
        Some(glyph)
    }
}
