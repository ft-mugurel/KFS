#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    Backspace,
    Tab,
    Enter,
    Space,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Digit0,
    Minus,
    Equal,
    LeftBracket,
    RightBracket,
    Backslash,
    Semicolon,
    Apostrophe,
    Grave,
    Comma,
    Dot,
    Slash,
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    LeftShift,
    RightShift,
    LeftCtrl,
    RightCtrl,
    LeftAlt,
    RightAlt,
    LeftSuper,
    RightSuper,
    AltGr,
    CapsLock,
    ArrowUp,
    ArrowDown,
    Home,
    PageUp,
    PageDown,
    End,
    ArrowLeft,
    ArrowRight,
    Delete,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: KeyCode,
    pub pressed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Modifiers(u16);

impl Modifiers {
    const SHIFT: u16 = 1 << 0;
    const CTRL: u16 = 1 << 1;
    const ALT: u16 = 1 << 2;
    const SUPER: u16 = 1 << 3;
    const CAPS_LOCK: u16 = 1 << 4;
    const ALT_GR: u16 = 1 << 5;

    pub const fn empty() -> Self {
        Self(0)
    }

    pub fn shift(self) -> bool {
        (self.0 & Self::SHIFT) != 0
    }

    pub fn ctrl(self) -> bool {
        (self.0 & Self::CTRL) != 0
    }

    pub fn alt(self) -> bool {
        (self.0 & Self::ALT) != 0
    }

    pub fn super_key(self) -> bool {
        (self.0 & Self::SUPER) != 0
    }

    pub fn caps_lock(self) -> bool {
        (self.0 & Self::CAPS_LOCK) != 0
    }

    pub fn alt_gr(self) -> bool {
        (self.0 & Self::ALT_GR) != 0
    }

    pub fn has_text_blocking_modifier(self) -> bool {
        self.ctrl() || self.alt() || self.super_key() || self.alt_gr()
    }

    fn set_flag(&mut self, flag: u16, enabled: bool) {
        if enabled {
            self.0 |= flag;
        } else {
            self.0 &= !flag;
        }
    }

    fn toggle_flag(&mut self, flag: u16) {
        self.0 ^= flag;
    }

    pub fn update_for_event(&mut self, event: KeyEvent) {
        match event.key {
            KeyCode::LeftShift | KeyCode::RightShift => {
                self.set_flag(Self::SHIFT, event.pressed);
            }
            KeyCode::LeftCtrl | KeyCode::RightCtrl => {
                self.set_flag(Self::CTRL, event.pressed);
            }
            KeyCode::LeftAlt | KeyCode::RightAlt => {
                self.set_flag(Self::ALT, event.pressed);
            }
            KeyCode::LeftSuper | KeyCode::RightSuper => {
                self.set_flag(Self::SUPER, event.pressed);
            }
            KeyCode::CapsLock if event.pressed => {
                self.toggle_flag(Self::CAPS_LOCK);
            }
            KeyCode::AltGr if event.pressed => {
                self.set_flag(Self::ALT_GR, event.pressed);
            }
            _ => {}
        }
    }
}

const SCANCODE_RELEASE_MASK: u8 = 0x80;
const SCANCODE_KEY_MASK: u8 = 0x7F;

pub fn decode_set1_scancode(scancode: u8, extended: bool) -> Option<KeyEvent> {
    let pressed = (scancode & SCANCODE_RELEASE_MASK) == 0;
    let key = scancode & SCANCODE_KEY_MASK;

    let keycode = if extended {
        match key {
            0x48 => KeyCode::ArrowUp,
            0x50 => KeyCode::ArrowDown,
            0x47 => KeyCode::Home,
            0x49 => KeyCode::PageUp,
            0x51 => KeyCode::PageDown,
            0x4B => KeyCode::ArrowLeft,
            0x4D => KeyCode::ArrowRight,
            0x4F => KeyCode::End,
            0x53 => KeyCode::Delete,
            0x1D => KeyCode::RightCtrl,
            0x38 => KeyCode::RightAlt,
            0x5B => KeyCode::LeftSuper,
            0x5C => KeyCode::RightSuper,
            0x5D => KeyCode::AltGr,
            _ => return None,
        }
    } else {
        match key {
            0x0E => KeyCode::Backspace,
            0x0F => KeyCode::Tab,
            0x1C => KeyCode::Enter,
            0x39 => KeyCode::Space,
            0x02 => KeyCode::Digit1,
            0x03 => KeyCode::Digit2,
            0x04 => KeyCode::Digit3,
            0x05 => KeyCode::Digit4,
            0x06 => KeyCode::Digit5,
            0x07 => KeyCode::Digit6,
            0x08 => KeyCode::Digit7,
            0x09 => KeyCode::Digit8,
            0x0A => KeyCode::Digit9,
            0x0B => KeyCode::Digit0,
            0x0C => KeyCode::Minus,
            0x0D => KeyCode::Equal,
            0x1A => KeyCode::LeftBracket,
            0x1B => KeyCode::RightBracket,
            0x2B => KeyCode::Backslash,
            0x27 => KeyCode::Semicolon,
            0x28 => KeyCode::Apostrophe,
            0x29 => KeyCode::Grave,
            0x33 => KeyCode::Comma,
            0x34 => KeyCode::Dot,
            0x35 => KeyCode::Slash,
            0x10 => KeyCode::Q,
            0x11 => KeyCode::W,
            0x12 => KeyCode::E,
            0x13 => KeyCode::R,
            0x14 => KeyCode::T,
            0x15 => KeyCode::Y,
            0x16 => KeyCode::U,
            0x17 => KeyCode::I,
            0x18 => KeyCode::O,
            0x19 => KeyCode::P,
            0x1E => KeyCode::A,
            0x1F => KeyCode::S,
            0x20 => KeyCode::D,
            0x21 => KeyCode::F,
            0x22 => KeyCode::G,
            0x23 => KeyCode::H,
            0x24 => KeyCode::J,
            0x25 => KeyCode::K,
            0x26 => KeyCode::L,
            0x2C => KeyCode::Z,
            0x2D => KeyCode::X,
            0x2E => KeyCode::C,
            0x2F => KeyCode::V,
            0x30 => KeyCode::B,
            0x31 => KeyCode::N,
            0x32 => KeyCode::M,
            0x2A => KeyCode::LeftShift,
            0x36 => KeyCode::RightShift,
            0x1D => KeyCode::LeftCtrl,
            0x38 => KeyCode::LeftAlt,
            0x5D => KeyCode::AltGr,
            0x3A => KeyCode::CapsLock,
            0x3B => KeyCode::F1,
            0x3C => KeyCode::F2,
            0x3D => KeyCode::F3,
            0x3E => KeyCode::F4,
            0x3F => KeyCode::F5,
            0x40 => KeyCode::F6,
            0x41 => KeyCode::F7,
            0x42 => KeyCode::F8,
            0x43 => KeyCode::F9,
            0x44 => KeyCode::F10,
            0x57 => KeyCode::F11,
            0x58 => KeyCode::F12,
            _ => return None,
        }
    };

    Some(KeyEvent { key: keycode, pressed })
}
