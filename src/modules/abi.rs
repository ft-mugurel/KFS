use crate::error::{KResult, KernelError};
pub use crate::interrupts::keyboard::types::{KeyCode, Modifiers};

pub const MODULE_ABI_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct ModuleId(pub(super) u32);

impl ModuleId {
    pub const INVALID: Self = Self(0);
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct ModuleStr {
    ptr: *const u8,
    len: usize,
}

impl ModuleStr {
    pub const fn new(value: &'static str) -> Self {
        Self { ptr: value.as_ptr(), len: value.len() }
    }

    pub(super) unsafe fn to_str(self) -> KResult<&'static str> {
        if self.ptr.is_null() || self.len == 0 {
            return Err(KernelError::Inval);
        }
        let bytes = unsafe { core::slice::from_raw_parts(self.ptr, self.len) };
        core::str::from_utf8(bytes).map_err(|_| KernelError::Inval)
    }
}

unsafe impl Sync for ModuleStr {}
unsafe impl Send for ModuleStr {}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct ModuleDescriptor {
    pub abi_version: u32,
    pub name: ModuleStr,
    pub version: u32,
    pub flags: u32,
    pub init: ModuleInitFn,
    pub destroy: ModuleDestroyFn,
    pub dependencies: *const ModuleStr,
    pub dependency_count: usize,
}

unsafe impl Sync for ModuleDescriptor {}
unsafe impl Send for ModuleDescriptor {}

impl ModuleDescriptor {
    pub const fn new(
        name: &'static str,
        version: u32,
        init: ModuleInitFn,
        destroy: ModuleDestroyFn,
        dependencies: &'static [ModuleStr],
    ) -> Self {
        Self {
            abi_version: MODULE_ABI_VERSION,
            name: ModuleStr::new(name),
            version,
            flags: 0,
            init,
            destroy,
            dependencies: dependencies.as_ptr(),
            dependency_count: dependencies.len(),
        }
    }
}

pub type ModuleInitFn = extern "C" fn(context: &KernelModuleContext) -> i32;
pub type ModuleDestroyFn = extern "C" fn(context: &KernelModuleContext);
pub type ModuleCallbackFn =
    extern "C" fn(context: &KernelModuleContext, event: &ModuleEvent, data: usize);

#[derive(Clone, Copy)]
#[repr(C)]
pub struct KernelModuleContext {
    pub module: ModuleId,
    pub api: &'static KernelModuleApi,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ModuleEventKind {
    CpuTick = 0,
    KeyPressed = 1,
    KeyReleased = 2,
}

impl ModuleEventKind {
    pub(super) const COUNT: usize = 3;

    pub(super) fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::CpuTick),
            1 => Some(Self::KeyPressed),
            2 => Some(Self::KeyReleased),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct KeyboardEvent {
    pub scancode: u16,
    pub key: KeyCode,
    pub pressed: bool,
    pub modifiers: Modifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C, u32)]
pub enum ModuleEvent {
    CpuTick { cpu_id: u32, ticks: u64 } = 0,
    KeyPressed(KeyboardEvent) = 1,
    KeyReleased(KeyboardEvent) = 2,
}

impl ModuleEvent {
    pub fn kind(&self) -> ModuleEventKind {
        match self {
            Self::CpuTick { .. } => ModuleEventKind::CpuTick,
            Self::KeyPressed(_) => ModuleEventKind::KeyPressed,
            Self::KeyReleased(_) => ModuleEventKind::KeyReleased,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct CallbackToken(pub(super) u32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct ClockReading {
    pub ticks: u64,
    pub frequency_hz: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct TimeValue {
    pub ticks: u64,
    pub frequency_hz: u32,
    pub uptime_ms: u64,
}

pub type TimeNowFn = extern "C" fn(out: &mut TimeValue) -> i32;
pub const TIME_NOW_EXPORT: &str = "time.now";
pub const TIME_NOW_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ConsoleOp {
    SwitchScreen = 0,
    NextScreen = 1,
    PreviousScreen = 2,
    ScrollUp = 3,
    ScrollDown = 4,
    ScrollTop = 5,
    ScrollBottom = 6,
    BigCursor = 7,
    SmallCursor = 8,
    CursorShape = 9,
    HideCursor = 10,
    ShowCursor = 11,
}

impl ConsoleOp {
    pub(super) fn from_raw(raw: u32) -> Option<Self> {
        const OPS: [ConsoleOp; 12] = [
            ConsoleOp::SwitchScreen,
            ConsoleOp::NextScreen,
            ConsoleOp::PreviousScreen,
            ConsoleOp::ScrollUp,
            ConsoleOp::ScrollDown,
            ConsoleOp::ScrollTop,
            ConsoleOp::ScrollBottom,
            ConsoleOp::BigCursor,
            ConsoleOp::SmallCursor,
            ConsoleOp::CursorShape,
            ConsoleOp::HideCursor,
            ConsoleOp::ShowCursor,
        ];
        OPS.get(raw as usize).copied()
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct KernelModuleApi {
    pub abi_version: u32,
    pub struct_size: u32,

    pub log: unsafe extern "C" fn(ModuleId, *const u8, usize) -> i32,

    pub alloc: extern "C" fn(ModuleId, usize, &mut *mut u8) -> i32,
    pub free: extern "C" fn(ModuleId, *mut u8) -> i32,

    pub callback_register:
        extern "C" fn(ModuleId, u32, ModuleCallbackFn, usize, &mut CallbackToken) -> i32,
    pub callback_unregister: extern "C" fn(ModuleId, CallbackToken) -> i32,

    pub export_register: unsafe extern "C" fn(ModuleId, *const u8, usize, usize, u32) -> i32,
    pub export_lookup: unsafe extern "C" fn(ModuleId, *const u8, usize, u32, &mut usize) -> i32,

    pub clock_read: extern "C" fn(&mut ClockReading) -> i32,
    pub time_now: extern "C" fn(&mut TimeValue) -> i32,

    pub tty_input: unsafe extern "C" fn(ModuleId, *const u8, usize) -> i32,
    pub console_control: extern "C" fn(ModuleId, u32, u32) -> i32,
    pub signal_foreground: extern "C" fn(ModuleId, u32) -> i32,
    pub request_shutdown: extern "C" fn(ModuleId) -> i32,
}

pub(super) fn code(result: KResult<()>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(error) => error as i32,
    }
}

fn check(code: i32) -> KResult<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(KernelError::from_code(code))
    }
}

#[expect(dead_code, reason = "module-facing ABI; called by loadable modules")]
impl KernelModuleContext {
    pub fn validate(&self) -> KResult<()> {
        if self.api.abi_version != MODULE_ABI_VERSION
            || (self.api.struct_size as usize) < core::mem::size_of::<KernelModuleApi>()
        {
            return Err(KernelError::NoExec);
        }
        Ok(())
    }

    pub fn log(&self, message: &str) {
        unsafe { (self.api.log)(self.module, message.as_ptr(), message.len()) };
    }

    pub fn alloc(&self, size: usize) -> KResult<*mut u8> {
        let mut ptr = core::ptr::null_mut();
        check((self.api.alloc)(self.module, size, &mut ptr))?;
        Ok(ptr)
    }

    pub fn free(&self, ptr: *mut u8) -> KResult<()> {
        check((self.api.free)(self.module, ptr))
    }

    pub fn register_callback(
        &self,
        kind: ModuleEventKind,
        callback: ModuleCallbackFn,
        data: usize,
    ) -> KResult<CallbackToken> {
        let mut token = CallbackToken(0);
        check((self.api.callback_register)(
            self.module,
            kind as u32,
            callback,
            data,
            &mut token,
        ))?;
        Ok(token)
    }

    pub fn unregister_callback(&self, token: CallbackToken) -> KResult<()> {
        check((self.api.callback_unregister)(self.module, token))
    }

    pub fn export(&self, name: &'static str, address: usize, version: u32) -> KResult<()> {
        check(unsafe {
            (self.api.export_register)(self.module, name.as_ptr(), name.len(), address, version)
        })
    }

    pub fn lookup(&self, name: &str, version: u32) -> KResult<usize> {
        let mut address = 0;
        check(unsafe {
            (self.api.export_lookup)(
                self.module,
                name.as_ptr(),
                name.len(),
                version,
                &mut address,
            )
        })?;
        Ok(address)
    }

    pub fn clock(&self) -> KResult<ClockReading> {
        let mut reading = ClockReading::default();
        check((self.api.clock_read)(&mut reading))?;
        Ok(reading)
    }

    pub fn time_now(&self) -> KResult<TimeValue> {
        let mut value = TimeValue::default();
        check((self.api.time_now)(&mut value))?;
        Ok(value)
    }

    pub fn tty_input(&self, bytes: &[u8]) -> KResult<()> {
        check(unsafe { (self.api.tty_input)(self.module, bytes.as_ptr(), bytes.len()) })
    }

    pub fn console(&self, op: ConsoleOp, arg: u32) -> KResult<()> {
        check((self.api.console_control)(self.module, op as u32, arg))
    }

    pub fn signal_foreground(&self, signal: u32) -> KResult<()> {
        check((self.api.signal_foreground)(self.module, signal))
    }

    pub fn request_shutdown(&self) -> KResult<()> {
        check((self.api.request_shutdown)(self.module))
    }
}
