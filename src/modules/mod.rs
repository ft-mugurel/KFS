mod abi;
mod api;
mod callbacks;
mod keyboard;
mod time;

pub use self::abi::{
    CallbackToken, KernelModuleContext, KeyboardEvent, MODULE_ABI_VERSION, ModuleDescriptor,
    ModuleEvent, ModuleEventKind, ModuleId, ModuleStr, TimeValue,
};
pub(crate) use self::callbacks::dispatch;

use self::abi::{TIME_NOW_EXPORT, TIME_NOW_VERSION, TimeNowFn};
use crate::{
    error::{KResult, KernelError},
    locks::Spinlock,
    paging, pr_err, pr_info, pr_warn,
};

const MAX_MODULES: usize = 16;
const MAX_DEPENDENCIES: usize = 32;
const MAX_EXPORTS: usize = 32;
const MAX_ALLOCATIONS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ModuleState {
    Registered,
    Creating,
    Initializing,
    Active,
    Destroying,
    Destroyed,
}

#[derive(Clone, Copy)]
struct ModuleRecord {
    name: &'static str,
    descriptor: &'static ModuleDescriptor,
    state: ModuleState,
    dependents: usize,
    references: usize,
}

#[derive(Clone, Copy)]
struct Dependency {
    consumer: ModuleId,
    provider: ModuleId,
}

#[derive(Clone, Copy)]
struct ExportRecord {
    name: &'static str,
    owner: ModuleId,
    address: usize,
    version: u32,
}

#[derive(Clone, Copy)]
struct Allocation {
    owner: ModuleId,
    ptr: *mut u8,
}

struct ModuleManager {
    modules: [Option<ModuleRecord>; MAX_MODULES],
    generations: [u16; MAX_MODULES],
    dependencies: [Option<Dependency>; MAX_DEPENDENCIES],
    exports: [Option<ExportRecord>; MAX_EXPORTS],
    allocations: [Option<Allocation>; MAX_ALLOCATIONS],
    callbacks: callbacks::CallbackTable,
}

unsafe impl Send for ModuleManager {}

impl ModuleId {
    fn new(slot: usize, generation: u16) -> Self {
        Self(((generation as u32) << 16) | (slot as u32 + 1))
    }

    fn slot(self) -> Option<usize> {
        let slot = (self.0 & 0xFFFF) as usize;
        (slot != 0 && slot <= MAX_MODULES).then(|| slot - 1)
    }

    fn generation(self) -> u16 {
        (self.0 >> 16) as u16
    }
}

impl ModuleManager {
    const fn new() -> Self {
        Self {
            modules: [None; MAX_MODULES],
            generations: [0; MAX_MODULES],
            dependencies: [None; MAX_DEPENDENCIES],
            exports: [None; MAX_EXPORTS],
            allocations: [None; MAX_ALLOCATIONS],
            callbacks: callbacks::CallbackTable::new(),
        }
    }

    fn find(&self, name: &str) -> Option<ModuleId> {
        self.modules.iter().enumerate().find_map(|(slot, record)| {
            record
                .filter(|record| record.name == name)
                .map(|_| ModuleId::new(slot, self.generations[slot]))
        })
    }

    fn record(&self, id: ModuleId) -> KResult<&ModuleRecord> {
        let slot = id.slot().ok_or(KernelError::NoEnt)?;
        if self.generations[slot] != id.generation() {
            return Err(KernelError::NoEnt);
        }
        self.modules[slot].as_ref().ok_or(KernelError::NoEnt)
    }

    fn record_mut(&mut self, id: ModuleId) -> KResult<&mut ModuleRecord> {
        let slot = id.slot().ok_or(KernelError::NoEnt)?;
        if self.generations[slot] != id.generation() {
            return Err(KernelError::NoEnt);
        }
        self.modules[slot].as_mut().ok_or(KernelError::NoEnt)
    }

    fn state(&self, id: ModuleId) -> Option<ModuleState> {
        self.record(id).ok().map(|record| record.state)
    }

    fn is_live(&self, id: ModuleId) -> bool {
        matches!(
            self.state(id),
            Some(ModuleState::Initializing | ModuleState::Active)
        )
    }

    fn remove(&mut self, id: ModuleId) {
        if let Some(slot) = id.slot() {
            self.modules[slot] = None;
            self.generations[slot] = self.generations[slot].wrapping_add(1);
        }
    }

    fn reaches(&self, from: ModuleId, target: ModuleId) -> bool {
        let mut stack = [ModuleId::INVALID; MAX_MODULES];
        let mut depth = 0;
        let mut visited = 0u32;
        stack[0] = from;
        depth += 1;
        while depth > 0 {
            depth -= 1;
            let node = stack[depth];
            if node == target {
                return true;
            }
            for edge in self.dependencies.iter().flatten() {
                let Some(slot) = edge.provider.slot() else {
                    continue;
                };
                if edge.consumer == node && visited & (1 << slot) == 0 {
                    visited |= 1 << slot;
                    stack[depth] = edge.provider;
                    depth += 1;
                }
            }
        }
        false
    }

    fn add_dependency(&mut self, consumer: ModuleId, provider: ModuleId) -> KResult<()> {
        if !self.is_live(consumer) && self.state(consumer) != Some(ModuleState::Creating) {
            return Err(KernelError::NoEnt);
        }
        if self.state(provider) != Some(ModuleState::Active) {
            return Err(if self.record(provider).is_ok() {
                KernelError::Busy
            } else {
                KernelError::NoEnt
            });
        }
        if self.reaches(provider, consumer) {
            return Err(KernelError::Deadlock);
        }
        if self
            .dependencies
            .iter()
            .flatten()
            .any(|edge| edge.consumer == consumer && edge.provider == provider)
        {
            return Ok(());
        }
        let slot = self
            .dependencies
            .iter()
            .position(Option::is_none)
            .ok_or(KernelError::NoSpc)?;
        self.dependencies[slot] = Some(Dependency { consumer, provider });
        self.record_mut(provider)?.dependents += 1;
        Ok(())
    }

    fn release_dependencies(&mut self, consumer: ModuleId) {
        for index in 0..MAX_DEPENDENCIES {
            let Some(edge) = self.dependencies[index] else {
                continue;
            };
            if edge.consumer == consumer {
                self.dependencies[index] = None;
                if let Ok(provider) = self.record_mut(edge.provider) {
                    provider.dependents -= 1;
                }
            }
        }
    }

    fn acquire_declared_dependencies(
        &mut self,
        consumer: ModuleId,
        descriptor: &ModuleDescriptor,
    ) -> KResult<()> {
        for index in 0..descriptor.dependency_count {
            let name = unsafe { (*descriptor.dependencies.add(index)).to_str()? };
            let provider = self.find(name).ok_or(KernelError::NoEnt)?;
            if provider == consumer {
                return Err(KernelError::Inval);
            }
            self.add_dependency(consumer, provider)?;
        }
        Ok(())
    }
}

static MANAGER: Spinlock<ModuleManager> = Spinlock::new(ModuleManager::new());

fn context_for(module: ModuleId) -> KernelModuleContext {
    KernelModuleContext { module, api: &api::API }
}

pub fn register(descriptor: &'static ModuleDescriptor) -> KResult<ModuleId> {
    if descriptor.abi_version != MODULE_ABI_VERSION {
        return Err(KernelError::NoExec);
    }
    if descriptor.flags != 0 || descriptor.dependency_count > MAX_DEPENDENCIES {
        return Err(KernelError::Inval);
    }
    let name = unsafe { descriptor.name.to_str()? };
    for index in 0..descriptor.dependency_count {
        unsafe { (*descriptor.dependencies.add(index)).to_str()? };
    }

    let mut manager = MANAGER.lock();
    if manager.find(name).is_some() {
        return Err(KernelError::Exist);
    }
    let slot = manager
        .modules
        .iter()
        .position(Option::is_none)
        .ok_or(KernelError::NoSpc)?;
    manager.modules[slot] = Some(ModuleRecord {
        name,
        descriptor,
        state: ModuleState::Registered,
        dependents: 0,
        references: 0,
    });
    Ok(ModuleId::new(slot, manager.generations[slot]))
}

pub fn find(name: &str) -> Option<ModuleId> {
    MANAGER.lock().find(name)
}

pub fn state(id: ModuleId) -> KResult<ModuleState> {
    MANAGER.lock().record(id).map(|record| record.state)
}

fn name_of(id: ModuleId) -> Option<&'static str> {
    MANAGER.lock().record(id).ok().map(|record| record.name)
}

pub fn create(id: ModuleId) -> KResult<()> {
    if callbacks::in_callback() {
        return Err(KernelError::Deadlock);
    }
    let (name, descriptor) = {
        let mut manager = MANAGER.lock();
        let record = *manager.record(id)?;
        if record.state != ModuleState::Registered {
            return Err(KernelError::Busy);
        }
        manager.record_mut(id)?.state = ModuleState::Creating;
        if let Err(error) = manager.acquire_declared_dependencies(id, record.descriptor) {
            manager.release_dependencies(id);
            manager.remove(id);
            pr_warn!(
                "module {}: dependencies unavailable: {:?}",
                record.name,
                error
            );
            return Err(error);
        }
        manager.record_mut(id)?.state = ModuleState::Initializing;
        (record.name, record.descriptor)
    };

    let status = (descriptor.init)(&context_for(id));
    if status != 0 {
        let error = KernelError::from_code(status);
        teardown(id, None);
        MANAGER.lock().remove(id);
        pr_warn!("module {}: init failed: {:?}", name, error);
        return Err(error);
    }
    MANAGER.lock().record_mut(id)?.state = ModuleState::Active;
    pr_info!("module {} v{}: active", name, descriptor.version);
    Ok(())
}

pub fn destroy(id: ModuleId) -> KResult<()> {
    if callbacks::in_callback() {
        return Err(KernelError::Deadlock);
    }
    let record = {
        let mut manager = MANAGER.lock();
        let record = manager.record_mut(id)?;
        if record.state != ModuleState::Active {
            return Err(KernelError::Inval);
        }
        if record.dependents != 0 || record.references != 0 {
            return Err(KernelError::Busy);
        }
        record.state = ModuleState::Destroying;
        *record
    };
    teardown(id, Some(record.descriptor));
    MANAGER.lock().record_mut(id)?.state = ModuleState::Destroyed;
    pr_info!("module {}: destroyed", record.name);
    Ok(())
}

fn teardown(id: ModuleId, descriptor: Option<&'static ModuleDescriptor>) {
    MANAGER
        .lock()
        .exports
        .iter_mut()
        .filter(|export| export.is_some_and(|export| export.owner == id))
        .for_each(|export| *export = None);

    callbacks::revoke_all(id);

    if let Some(descriptor) = descriptor {
        (descriptor.destroy)(&context_for(id));
    }

    let mut owned = [core::ptr::null_mut(); MAX_ALLOCATIONS];
    let mut count = 0;
    {
        let mut manager = MANAGER.lock();
        for allocation in &mut manager.allocations {
            if let Some(entry) = allocation.filter(|entry| entry.owner == id) {
                owned[count] = entry.ptr;
                count += 1;
                *allocation = None;
            }
        }
    }
    for ptr in &owned[..count] {
        let _ = paging::kfree(*ptr);
    }

    MANAGER.lock().release_dependencies(id);
}

pub fn unregister_module(id: ModuleId) -> KResult<()> {
    let mut manager = MANAGER.lock();
    match manager.record(id)?.state {
        ModuleState::Registered | ModuleState::Destroyed => {
            manager.remove(id);
            Ok(())
        }
        _ => Err(KernelError::Busy),
    }
}

pub fn acquire(id: ModuleId) -> KResult<()> {
    let mut manager = MANAGER.lock();
    let record = manager.record_mut(id)?;
    if record.state != ModuleState::Active {
        return Err(KernelError::Busy);
    }
    record.references += 1;
    Ok(())
}

pub fn release(id: ModuleId) -> KResult<()> {
    let mut manager = MANAGER.lock();
    let record = manager.record_mut(id)?;
    if record.references == 0 {
        return Err(KernelError::Inval);
    }
    record.references -= 1;
    Ok(())
}

pub fn add_dependency(consumer: ModuleId, provider: ModuleId) -> KResult<()> {
    if consumer == provider {
        return Err(KernelError::Inval);
    }
    MANAGER.lock().add_dependency(consumer, provider)
}

fn export_register(
    owner: ModuleId,
    name: &'static str,
    address: usize,
    version: u32,
) -> KResult<()> {
    let mut manager = MANAGER.lock();
    if manager.state(owner) != Some(ModuleState::Initializing) {
        return Err(KernelError::Perm);
    }
    if address == 0 {
        return Err(KernelError::Inval);
    }
    if manager
        .exports
        .iter()
        .flatten()
        .any(|export| export.name == name)
    {
        return Err(KernelError::Exist);
    }
    let slot = manager
        .exports
        .iter()
        .position(Option::is_none)
        .ok_or(KernelError::NoSpc)?;
    manager.exports[slot] = Some(ExportRecord { name, owner, address, version });
    Ok(())
}

fn find_export(manager: &ModuleManager, name: &str, version: u32) -> KResult<ExportRecord> {
    let export = *manager
        .exports
        .iter()
        .flatten()
        .find(|export| export.name == name)
        .ok_or(KernelError::NoDev)?;
    if export.version != version {
        return Err(KernelError::NoExec);
    }
    Ok(export)
}

fn export_lookup(consumer: ModuleId, name: &str, version: u32) -> KResult<usize> {
    let mut manager = MANAGER.lock();
    if !manager.is_live(consumer) {
        return Err(KernelError::Perm);
    }
    let export = find_export(&manager, name, version)?;
    if export.owner != consumer {
        manager.add_dependency(consumer, export.owner)?;
    }
    Ok(export.address)
}

fn with_export<R>(name: &str, version: u32, call: impl FnOnce(usize) -> R) -> KResult<R> {
    let export = {
        let mut manager = MANAGER.lock();
        let export = find_export(&manager, name, version)?;
        let owner = manager.record_mut(export.owner)?;
        if owner.state != ModuleState::Active {
            return Err(KernelError::NoDev);
        }
        owner.references += 1;
        export
    };
    let result = call(export.address);
    let _ = release(export.owner);
    Ok(result)
}

pub fn time_now() -> KResult<TimeValue> {
    with_export(TIME_NOW_EXPORT, TIME_NOW_VERSION, |address| {
        let now = unsafe { core::mem::transmute::<usize, TimeNowFn>(address) };
        let mut value = TimeValue::default();
        match now(&mut value) {
            0 => Ok(value),
            status => Err(KernelError::from_code(status)),
        }
    })?
}

fn module_alloc(owner: ModuleId, size: usize) -> KResult<*mut u8> {
    if callbacks::in_callback() {
        return Err(KernelError::Perm);
    }
    if size == 0 {
        return Err(KernelError::Inval);
    }
    if !MANAGER.lock().is_live(owner) {
        return Err(KernelError::Perm);
    }
    let ptr = paging::kmalloc(size)?;
    let mut manager = MANAGER.lock();
    let Some(slot) = manager.allocations.iter().position(Option::is_none) else {
        drop(manager);
        let _ = paging::kfree(ptr);
        return Err(KernelError::NoSpc);
    };
    manager.allocations[slot] = Some(Allocation { owner, ptr });
    Ok(ptr)
}

fn module_free(owner: ModuleId, ptr: *mut u8) -> KResult<()> {
    if callbacks::in_callback() {
        return Err(KernelError::Perm);
    }
    let mut manager = MANAGER.lock();
    let slot = manager
        .allocations
        .iter()
        .position(|entry| entry.is_some_and(|entry| entry.owner == owner && entry.ptr == ptr))
        .ok_or(KernelError::Inval)?;
    manager.allocations[slot] = None;
    drop(manager);
    paging::kfree(ptr)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceCounts {
    pub modules: usize,
    pub callbacks: usize,
    pub dependencies: usize,
    pub exports: usize,
    pub allocations: usize,
}

pub fn resource_counts() -> ResourceCounts {
    let manager = MANAGER.lock();
    ResourceCounts {
        modules: manager.modules.iter().flatten().count(),
        callbacks: manager.callbacks.len(),
        dependencies: manager.dependencies.iter().flatten().count(),
        exports: manager.exports.iter().flatten().count(),
        allocations: manager.allocations.iter().flatten().count(),
    }
}

pub fn dispatch_cpu_tick(cpu_id: u32) {
    dispatch(ModuleEvent::CpuTick {
        cpu_id,
        ticks: crate::interrupts::timer::get_ticks() as u64,
    });
}

struct BuiltinModule {
    descriptor: &'static ModuleDescriptor,
    autostart: bool,
}

static BUILTIN_MODULES: [BuiltinModule; 2] = [
    BuiltinModule { descriptor: &time::DESCRIPTOR, autostart: true },
    BuiltinModule { descriptor: &keyboard::DESCRIPTOR, autostart: true },
];

pub const fn builtin_count() -> usize {
    BUILTIN_MODULES.len()
}

pub fn builtin_name(index: usize) -> Option<&'static str> {
    let builtin = BUILTIN_MODULES.get(index)?;
    unsafe { builtin.descriptor.name.to_str() }.ok()
}

pub fn builtin_state(index: usize) -> Option<ModuleState> {
    find(builtin_name(index)?).and_then(|id| state(id).ok())
}

pub fn set_builtin_enabled(index: usize, enabled: bool) -> KResult<()> {
    let builtin = BUILTIN_MODULES.get(index).ok_or(KernelError::NoEnt)?;
    let name = builtin_name(index).ok_or(KernelError::Inval)?;
    let existing = find(name);
    let current = existing.map(state).transpose()?;

    if !enabled {
        return match (existing, current) {
            (Some(id), Some(ModuleState::Active)) => destroy(id),
            (_, None | Some(ModuleState::Registered | ModuleState::Destroyed)) => Ok(()),
            _ => Err(KernelError::Busy),
        };
    }

    let id = match (existing, current) {
        (_, Some(ModuleState::Active)) => return Ok(()),
        (Some(id), Some(ModuleState::Registered)) => id,
        (Some(id), Some(ModuleState::Destroyed)) => {
            unregister_module(id)?;
            register(builtin.descriptor)?
        }
        (None, _) => register(builtin.descriptor)?,
        _ => return Err(KernelError::Busy),
    };
    create(id)
}

#[unsafe(link_section = ".init.text")]
pub fn init_modules() {
    let mut registered = [None; BUILTIN_MODULES.len()];
    for (index, builtin) in BUILTIN_MODULES.iter().enumerate() {
        if !builtin.autostart {
            continue;
        }
        match register(builtin.descriptor) {
            Ok(id) => registered[index] = Some(id),
            Err(error) => pr_err!("module #{}: registration failed: {:?}", index, error),
        }
    }

    let mut active = 0;
    for id in registered.into_iter().flatten() {
        if create(id).is_ok() {
            active += 1;
        }
    }
    let autostart = BUILTIN_MODULES
        .iter()
        .filter(|builtin| builtin.autostart)
        .count();
    pr_info!(
        "modules: {}/{} built-in modules active ({} disabled)",
        active,
        autostart,
        BUILTIN_MODULES.len() - autostart
    );
}

crate::device_initcall!(init_modules);
