#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct KeyEvent {
    pub key: KeyCode,
    pub pressed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Modifiers(pub(crate) u16);

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
    pub(crate) base: char,
    pub(crate) lvl2: char,
    pub(crate) is_letter: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputMode {
    EventDriven = 0,
    Blocking = 1,
}
