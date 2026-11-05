use core::sync::atomic::{AtomicU32, Ordering};

use crate::error::KernelError;
use crate::modules::{
    self, CallbackToken, KernelModuleContext, ModuleDescriptor, ModuleEvent, ModuleEventKind,
    ModuleId, ModuleStr,
};
use crate::{locks::Spinlock, pr_err, pr_info};

extern "C" fn init_ok(context: &KernelModuleContext) -> i32 {
    match context.validate() {
        Ok(()) => 0,
        Err(error) => error as i32,
    }
}

extern "C" fn destroy_noop(_: &KernelModuleContext) {}

extern "C" fn noop_callback(_: &KernelModuleContext, _: &ModuleEvent, _: usize) {}

extern "C" fn init_fail(context: &KernelModuleContext) -> i32 {
    let acquired = context
        .register_callback(ModuleEventKind::CpuTick, noop_callback, 0)
        .is_ok()
        && context.alloc(64).is_ok()
        && context
            .export("probe.fail.symbol", init_fail as *const () as usize, 1)
            .is_ok()
        && context.lookup("probe.provider.symbol", 1).is_ok();
    if acquired {
        KernelError::IO as i32
    } else {
        KernelError::Inval as i32
    }
}

extern "C" fn init_provider(context: &KernelModuleContext) -> i32 {
    match context.export(
        "probe.provider.symbol",
        init_provider as *const () as usize,
        1,
    ) {
        Ok(()) => 0,
        Err(error) => error as i32,
    }
}

extern "C" fn init_lookup(context: &KernelModuleContext) -> i32 {
    if context.lookup("probe.provider.symbol", 2).is_ok() {
        return KernelError::Inval as i32;
    }
    match context.lookup("probe.provider.symbol", 1) {
        Ok(_) => 0,
        Err(error) => error as i32,
    }
}

const PROBE_CPU: u32 = u32::MAX;

static TICKS_SEEN: AtomicU32 = AtomicU32::new(0);
static CALLBACK_TOKEN: Spinlock<Option<CallbackToken>> = Spinlock::new(None);
static CALLBACK_RESULTS: Spinlock<(i32, i32)> = Spinlock::new((0, 0));

extern "C" fn counting_callback(context: &KernelModuleContext, event: &ModuleEvent, data: usize) {
    if let ModuleEvent::CpuTick { cpu_id: PROBE_CPU, .. } = *event {
        TICKS_SEEN.fetch_add(data as u32, Ordering::SeqCst);
        let destroy_self = match modules::destroy(context.module) {
            Ok(()) => 0,
            Err(error) => error as i32,
        };
        let foreign_unregister = match *CALLBACK_TOKEN.lock() {
            Some(token) => (context.api.callback_unregister)(ModuleId::INVALID, token),
            None => 0,
        };
        *CALLBACK_RESULTS.lock() = (destroy_self, foreign_unregister);
    }
}

extern "C" fn init_callbacks(context: &KernelModuleContext) -> i32 {
    match context.register_callback(ModuleEventKind::CpuTick, counting_callback, 1) {
        Ok(token) => {
            *CALLBACK_TOKEN.lock() = Some(token);
            0
        }
        Err(error) => error as i32,
    }
}

static BAD_ABI: ModuleDescriptor = ModuleDescriptor {
    abi_version: modules::MODULE_ABI_VERSION + 1,
    ..ModuleDescriptor::new("probe.bad_abi", 1, init_ok, destroy_noop, &[])
};
static DUPLICATE: ModuleDescriptor =
    ModuleDescriptor::new("probe.dup", 1, init_ok, destroy_noop, &[]);
static FAILING: ModuleDescriptor =
    ModuleDescriptor::new("probe.fail", 1, init_fail, destroy_noop, &[]);
static PROVIDER: ModuleDescriptor =
    ModuleDescriptor::new("probe.provider", 1, init_provider, destroy_noop, &[]);
static CONSUMER: ModuleDescriptor = ModuleDescriptor::new(
    "probe.consumer",
    1,
    init_ok,
    destroy_noop,
    &[ModuleStr::new("probe.provider")],
);
static PARTIAL_DEPENDENCIES: ModuleDescriptor = ModuleDescriptor::new(
    "probe.partial",
    1,
    init_ok,
    destroy_noop,
    &[
        ModuleStr::new("probe.provider"),
        ModuleStr::new("probe.missing"),
    ],
);
static LOOKUP: ModuleDescriptor =
    ModuleDescriptor::new("probe.lookup", 1, init_lookup, destroy_noop, &[]);
static CALLBACKS: ModuleDescriptor =
    ModuleDescriptor::new("probe.callbacks", 1, init_callbacks, destroy_noop, &[]);

struct Probe {
    passed: u32,
    failed: u32,
}

impl Probe {
    fn check(&mut self, name: &str, ok: bool) {
        if ok {
            self.passed += 1;
        } else {
            self.failed += 1;
            pr_err!("modules_boot_probe: FAILED {}", name);
        }
    }
}

fn tick() {
    modules::dispatch(ModuleEvent::CpuTick { cpu_id: PROBE_CPU, ticks: 0 });
}

#[unsafe(link_section = ".init.text")]
pub(crate) fn modules_boot_probe() {
    let mut probe = Probe { passed: 0, failed: 0 };
    let baseline = modules::resource_counts();

    probe.check(
        "abi mismatch rejected",
        modules::register(&BAD_ABI) == Err(KernelError::NoExec),
    );

    let dup = modules::register(&DUPLICATE);
    probe.check("register", dup.is_ok());
    probe.check(
        "duplicate rejected",
        modules::register(&DUPLICATE) == Err(KernelError::Exist),
    );
    if let Ok(id) = dup {
        probe.check(
            "unregister never-created",
            modules::unregister_module(id).is_ok(),
        );
        probe.check(
            "stale id rejected",
            modules::state(id) == Err(KernelError::NoEnt),
        );
    }

    let consumer = modules::register(&CONSUMER).unwrap_or(ModuleId::INVALID);
    probe.check(
        "missing dependency",
        modules::create(consumer) == Err(KernelError::NoEnt),
    );
    probe.check(
        "failed create removed",
        modules::find("probe.consumer").is_none(),
    );

    let provider = modules::register(&PROVIDER).unwrap_or(ModuleId::INVALID);
    probe.check("provider active", modules::create(provider).is_ok());

    let partial = modules::register(&PARTIAL_DEPENDENCIES).unwrap_or(ModuleId::INVALID);
    probe.check(
        "partial dependencies",
        modules::create(partial) == Err(KernelError::NoEnt),
    );

    let before_fail = modules::resource_counts();
    let failing = modules::register(&FAILING).unwrap_or(ModuleId::INVALID);
    probe.check(
        "init failure reported",
        modules::create(failing) == Err(KernelError::IO),
    );
    probe.check(
        "init failure rolled back",
        modules::resource_counts() == before_fail,
    );

    let consumer = modules::register(&CONSUMER).unwrap_or(ModuleId::INVALID);
    probe.check("consumer active", modules::create(consumer).is_ok());
    probe.check(
        "provider pinned by consumer",
        modules::destroy(provider) == Err(KernelError::Busy),
    );
    probe.check(
        "cycle rejected",
        modules::add_dependency(provider, consumer) == Err(KernelError::Deadlock),
    );
    probe.check("consumer destroyed", modules::destroy(consumer).is_ok());
    probe.check(
        "consumer unregistered",
        modules::unregister_module(consumer).is_ok(),
    );

    let lookup = modules::register(&LOOKUP).unwrap_or(ModuleId::INVALID);
    probe.check("lookup active", modules::create(lookup).is_ok());
    probe.check(
        "provider pinned by lookup",
        modules::destroy(provider) == Err(KernelError::Busy),
    );
    probe.check("lookup destroyed", modules::destroy(lookup).is_ok());
    let _ = modules::unregister_module(lookup);

    probe.check("acquire", modules::acquire(provider).is_ok());
    probe.check(
        "referenced provider busy",
        modules::destroy(provider) == Err(KernelError::Busy),
    );
    probe.check("release", modules::release(provider).is_ok());
    probe.check(
        "unbalanced release",
        modules::release(provider) == Err(KernelError::Inval),
    );
    probe.check("provider destroyed", modules::destroy(provider).is_ok());
    probe.check(
        "destroyed twice",
        modules::destroy(provider) == Err(KernelError::Inval),
    );
    let _ = modules::unregister_module(provider);

    let callbacks = modules::register(&CALLBACKS).unwrap_or(ModuleId::INVALID);
    probe.check("callbacks active", modules::create(callbacks).is_ok());
    tick();
    probe.check("callback delivered", TICKS_SEEN.load(Ordering::SeqCst) == 1);
    let (destroy_self, foreign_unregister) = *CALLBACK_RESULTS.lock();
    probe.check(
        "destroy from callback refused",
        destroy_self == KernelError::Deadlock as i32,
    );
    probe.check(
        "foreign unregister refused",
        foreign_unregister == KernelError::Perm as i32,
    );
    probe.check("callbacks destroyed", modules::destroy(callbacks).is_ok());
    tick();
    probe.check(
        "no callback after destroy",
        TICKS_SEEN.load(Ordering::SeqCst) == 1,
    );
    let _ = modules::unregister_module(callbacks);

    probe.check(
        "no leaked resources",
        modules::resource_counts() == baseline,
    );

    let time = modules::time_now();
    probe.check(
        "time module answers",
        time.is_ok_and(|now| now.frequency_hz > 0),
    );

    if probe.failed == 0 {
        pr_info!("modules_boot_probe: all {} checks passed", probe.passed);
    } else {
        pr_err!(
            "modules_boot_probe: {} of {} checks failed",
            probe.failed,
            probe.passed + probe.failed
        );
    }
}

crate::late_initcall!(modules_boot_probe);
