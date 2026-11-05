use super::{
    MANAGER,
    abi::{
        CallbackToken, ClockReading, ConsoleOp, KernelModuleApi, MODULE_ABI_VERSION,
        ModuleCallbackFn, ModuleEventKind, ModuleId, TimeValue, code,
    },
    callbacks,
};
use crate::{
    error::{KResult, KernelError},
    interrupts::timer,
    pr_info,
    vga::text_mod as console,
};

pub(super) static API: KernelModuleApi = KernelModuleApi {
    abi_version: MODULE_ABI_VERSION,
    struct_size: core::mem::size_of::<KernelModuleApi>() as u32,
    log,
    alloc,
    free,
    callback_register,
    callback_unregister,
    export_register,
    export_lookup,
    clock_read,
    time_now,
    tty_input,
    console_control,
    signal_foreground,
    request_shutdown,
};

unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> KResult<&'a [u8]> {
    if ptr.is_null() {
        return if len == 0 { Ok(&[]) } else { Err(KernelError::Fault) };
    }
    Ok(unsafe { core::slice::from_raw_parts(ptr, len) })
}

unsafe fn module_str(ptr: *const u8, len: usize) -> KResult<&'static str> {
    let bytes = unsafe { bytes(ptr, len)? };
    core::str::from_utf8(bytes).map_err(|_| KernelError::Inval)
}

fn require_live(module: ModuleId) -> KResult<()> {
    if MANAGER.lock().is_live(module) {
        Ok(())
    } else {
        Err(KernelError::Perm)
    }
}

unsafe extern "C" fn log(module: ModuleId, ptr: *const u8, len: usize) -> i32 {
    code((|| {
        let message = unsafe { module_str(ptr, len)? };
        let name = super::name_of(module).ok_or(KernelError::Perm)?;
        pr_info!("[{}] {}", name, message);
        Ok(())
    })())
}

extern "C" fn alloc(module: ModuleId, size: usize, out: &mut *mut u8) -> i32 {
    code(super::module_alloc(module, size).map(|ptr| *out = ptr))
}

extern "C" fn free(module: ModuleId, ptr: *mut u8) -> i32 {
    code(super::module_free(module, ptr))
}

extern "C" fn callback_register(
    module: ModuleId,
    kind: u32,
    callback: ModuleCallbackFn,
    data: usize,
    out: &mut CallbackToken,
) -> i32 {
    code((|| {
        let kind = ModuleEventKind::from_raw(kind).ok_or(KernelError::Inval)?;
        *out = callbacks::register(module, kind, callback, data)?;
        Ok(())
    })())
}

extern "C" fn callback_unregister(module: ModuleId, token: CallbackToken) -> i32 {
    code(callbacks::unregister(module, token))
}

unsafe extern "C" fn export_register(
    module: ModuleId,
    name: *const u8,
    name_len: usize,
    address: usize,
    version: u32,
) -> i32 {
    if callbacks::in_callback() {
        return KernelError::Perm as i32;
    }
    code(
        unsafe { module_str(name, name_len) }
            .and_then(|name| super::export_register(module, name, address, version)),
    )
}

unsafe extern "C" fn export_lookup(
    module: ModuleId,
    name: *const u8,
    name_len: usize,
    version: u32,
    out: &mut usize,
) -> i32 {
    code(
        unsafe { module_str(name, name_len) }
            .and_then(|name| super::export_lookup(module, name, version))
            .map(|address| *out = address),
    )
}

extern "C" fn clock_read(out: &mut ClockReading) -> i32 {
    *out = ClockReading {
        ticks: timer::get_ticks() as u64,
        frequency_hz: timer::tick_frequency_hz(),
    };
    0
}

extern "C" fn time_now(out: &mut TimeValue) -> i32 {
    code(super::time_now().map(|value| *out = value))
}

unsafe extern "C" fn tty_input(module: ModuleId, ptr: *const u8, len: usize) -> i32 {
    code((|| {
        require_live(module)?;
        for &byte in unsafe { bytes(ptr, len)? } {
            crate::tty::push_char(byte as char);
        }
        Ok(())
    })())
}

extern "C" fn console_control(module: ModuleId, op: u32, arg: u32) -> i32 {
    code((|| {
        require_live(module)?;
        match ConsoleOp::from_raw(op).ok_or(KernelError::Inval)? {
            ConsoleOp::SwitchScreen => {
                if arg as usize >= crate::startup_config::vga::VIRTUAL_SCREENS {
                    return Err(KernelError::Inval);
                }
                console::switch_screen(arg as usize);
            }
            ConsoleOp::NextScreen => console::switch_to_next_screen(),
            ConsoleOp::PreviousScreen => console::switch_to_previous_screen(),
            ConsoleOp::ScrollUp => console::scroll_view_up(),
            ConsoleOp::ScrollDown => console::scroll_view_down(),
            ConsoleOp::ScrollTop => console::scroll_view_to_top(),
            ConsoleOp::ScrollBottom => console::scroll_view_to_bottom(),
            ConsoleOp::BigCursor => console::set_big_cursor(),
            ConsoleOp::SmallCursor => console::set_small_cursor(),
            ConsoleOp::CursorShape => console::set_cursor_shape(arg as u8, (arg >> 8) as u8),
            ConsoleOp::HideCursor => console::disable_cursor(),
            ConsoleOp::ShowCursor => console::enable_cursor(),
        }
        Ok(())
    })())
}

extern "C" fn signal_foreground(module: ModuleId, signal: u32) -> i32 {
    code((|| {
        require_live(module)?;
        if signal == 0 || signal as usize >= crate::sched::MAX_SIGNALS {
            return Err(KernelError::Inval);
        }
        crate::tty::signal_foreground(signal as u8);
        Ok(())
    })())
}

extern "C" fn request_shutdown(module: ModuleId) -> i32 {
    code(require_live(module).map(|()| unsafe { crate::smp::ipi::request_shutdown() }))
}
