mod api;
mod character_map;
mod init;
mod keycode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct KeyEvent {
    pub key: KeyCode,
    pub pressed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Modifiers(u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyCode {
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

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(crate) enum KeyboardLayout {
    UsQwerty = 0,
    TrQwerty = 1,
    GbQwerty = 2,

    LayoutEnd = 3,
}

#[derive(Clone, Copy)]
pub(crate) struct Glyph {
    base: char,
    lvl2: char,
    is_letter: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputMode {
    EventDriven = 0,
    Blocking = 1,
}

pub(crate) use api::{get_input_mode, get_line, push_char};
pub(crate) use character_map::{keycode_to_char, set_layout, toggle_layout};
pub(crate) use init::init_keyboard;
pub(crate) use keycode::decode_set1_scancode;
