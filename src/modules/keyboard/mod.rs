mod character_map;

use self::character_map::{keycode_to_char, toggle_layout};
use super::abi::{
    ConsoleOp, KernelModuleContext, KeyCode, KeyboardEvent, ModuleDescriptor, ModuleEvent,
    ModuleEventKind, code,
};
use crate::error::KResult;

const SIGINT: u32 = 2;

pub(super) static DESCRIPTOR: ModuleDescriptor =
    ModuleDescriptor::new("keyboard", 2, init, destroy, &[]);

extern "C" fn init(context: &KernelModuleContext) -> i32 {
    code(try_init(context))
}

fn try_init(context: &KernelModuleContext) -> KResult<()> {
    context.validate()?;
    context.register_callback(ModuleEventKind::KeyPressed, on_key, 0)?;
    context.register_callback(ModuleEventKind::KeyReleased, on_key, 0)?;
    Ok(())
}

extern "C" fn destroy(_: &KernelModuleContext) {}

extern "C" fn on_key(context: &KernelModuleContext, event: &ModuleEvent, _: usize) {
    match *event {
        ModuleEvent::KeyPressed(key) => handle_key_press(context, key),
        ModuleEvent::KeyReleased(_) | ModuleEvent::CpuTick { .. } => {}
    }
}

fn handle_key_press(context: &KernelModuleContext, event: KeyboardEvent) {
    let modifiers = event.modifiers;

    if modifiers.shift()
        && modifiers.alt()
        && (event.key == KeyCode::LeftShift || event.key == KeyCode::LeftAlt)
    {
        toggle_layout();
        return;
    }

    let console = |op: ConsoleOp, arg: u32| {
        let _ = context.console(op, arg);
    };

    match event.key {
        KeyCode::Delete if modifiers.ctrl() && modifiers.alt() => {
            let _ = context.request_shutdown();
            return;
        }
        KeyCode::C if modifiers.ctrl() => {
            let _ = context.signal_foreground(SIGINT);
            return;
        }
        KeyCode::F1 => return console(ConsoleOp::SwitchScreen, 0),
        KeyCode::F2 => return console(ConsoleOp::SwitchScreen, 1),
        KeyCode::F3 => return console(ConsoleOp::SwitchScreen, 2),
        KeyCode::F4 => return console(ConsoleOp::SwitchScreen, 3),
        KeyCode::F5 => return console(ConsoleOp::SwitchScreen, 4),
        KeyCode::F6 => return console(ConsoleOp::SwitchScreen, 5),
        KeyCode::F7 => return console(ConsoleOp::BigCursor, 0),
        KeyCode::F8 => return console(ConsoleOp::SmallCursor, 0),
        KeyCode::F9 => return console(ConsoleOp::CursorShape, 15 << 8),
        KeyCode::F10 => return console(ConsoleOp::HideCursor, 0),
        KeyCode::F11 => return console(ConsoleOp::ShowCursor, 0),
        KeyCode::PageUp => return console(ConsoleOp::ScrollTop, 0),
        KeyCode::PageDown => return console(ConsoleOp::ScrollBottom, 0),
        _ => {}
    }

    let input = |bytes: &[u8]| {
        let _ = context.tty_input(bytes);
    };

    match event.key {
        KeyCode::Backspace => return input(b"\x08"),
        KeyCode::Enter => return input(b"\n"),
        KeyCode::Tab => return input(b"\t"),
        _ => {}
    }

    if let Some(character) = keycode_to_char(event.key, modifiers) {
        let mut buffer = [0; 4];
        input(character.encode_utf8(&mut buffer).as_bytes());
        return;
    }

    match event.key {
        KeyCode::ArrowUp if modifiers.shift() => console(ConsoleOp::ScrollUp, 0),
        KeyCode::ArrowUp => input(b"\x1b[A"),
        KeyCode::ArrowDown if modifiers.shift() => console(ConsoleOp::ScrollDown, 0),
        KeyCode::ArrowDown => input(b"\x1b[B"),
        KeyCode::ArrowLeft if modifiers.shift() => console(ConsoleOp::PreviousScreen, 0),
        KeyCode::ArrowLeft => input(b"\x1b[D"),
        KeyCode::ArrowRight if modifiers.shift() => console(ConsoleOp::NextScreen, 0),
        KeyCode::ArrowRight => input(b"\x1b[C"),
        KeyCode::Home => input(b"\x1b[H"),
        KeyCode::End => input(b"\x1b[F"),
        KeyCode::Delete => input(b"\x1b[3~"),
        _ => {}
    }
}
