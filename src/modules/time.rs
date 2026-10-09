use super::{KernelModuleContext, ModuleDescriptor, TimeValue, MODULE_ABI_VERSION};
use crate::error::KResult;

pub(crate) static DESCRIPTOR: ModuleDescriptor = ModuleDescriptor {
    name: "time",
    version: 1,
    abi_version: MODULE_ABI_VERSION,
    flags: 0,
    init,
    destroy,
    dependencies: &[],
};

fn init(context: &KernelModuleContext) -> KResult<()> {
    context.api.validate()?;
    (context.api.log)(context.module, "time module received versioned API");
    #[allow(function_casts_as_integer)]
    (context.api.export_register)(context.module, "time.now", now as usize)
}

fn destroy(_: &KernelModuleContext) {}

fn now() -> TimeValue {
    TimeValue {
        ticks: crate::interrupts::timer::get_ticks() as u64,
        frequency_hz: crate::startup_config::power::CONFIG_HZ,
    }
}
