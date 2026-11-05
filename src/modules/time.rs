use core::sync::atomic::{AtomicPtr, Ordering};

use super::abi::{
    ClockReading, KernelModuleApi, KernelModuleContext, ModuleDescriptor, TimeNowFn, TimeValue,
    TIME_NOW_EXPORT, TIME_NOW_VERSION,
};
use crate::error::{KResult, KernelError};

pub(super) static DESCRIPTOR: ModuleDescriptor = ModuleDescriptor::new("time", 2, init, destroy, &[]);

static API: AtomicPtr<KernelModuleApi> = AtomicPtr::new(core::ptr::null_mut());

extern "C" fn init(context: &KernelModuleContext) -> i32 {
    super::abi::code(try_init(context))
}

fn try_init(context: &KernelModuleContext) -> KResult<()> {
    context.validate()?;
    API.store(context.api as *const _ as *mut _, Ordering::Release);
    let now: TimeNowFn = now;
    context.export(TIME_NOW_EXPORT, now as *const () as usize, TIME_NOW_VERSION)
}

extern "C" fn destroy(_: &KernelModuleContext) {
    API.store(core::ptr::null_mut(), Ordering::Release);
}

extern "C" fn now(out: &mut TimeValue) -> i32 {
    let Some(api) = (unsafe { API.load(Ordering::Acquire).as_ref() }) else {
        return KernelError::NoDev as i32;
    };
    let mut clock = ClockReading::default();
    let status = (api.clock_read)(&mut clock);
    if status != 0 {
        return status;
    }
    *out = TimeValue {
        ticks: clock.ticks,
        frequency_hz: clock.frequency_hz,
        uptime_ms: clock.ticks * 1000 / clock.frequency_hz.max(1) as u64,
    };
    0
}
