#![allow(dead_code)]

pub(crate) mod keyboard;
mod time;

use crate::{
    error::{KResult, KernelError},
    locks::Spinlock,
    paging,
    pr_debug, pr_err, pr_info, pr_warn,
};
use self::keyboard::types::{KeyEvent, Modifiers};

pub const MODULE_ABI_VERSION: u32 = 1;
const MAX_MODULES: usize = 16;
const MAX_CALLBACKS: usize = 64;
const MAX_DEPENDENCIES: usize = 32;
const MAX_EXPORTS: usize = 32;
const MAX_ALLOCATIONS: usize = 128;
const MAX_DEFERRED_EVENTS: usize = 32;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleId(u16);

impl ModuleId {
    pub const INVALID: Self = Self(0);
    fn index(self) -> usize {
        self.0.saturating_sub(1) as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleEvent {
    CpuTick { cpu_id: u32 },
    KeyPressed(KeyEvent, Modifiers),
    KeyReleased(KeyEvent, Modifiers),
}

pub type ModuleCallback = fn(ModuleId, ModuleEvent, &KernelModuleApi);
pub type ModuleInit = fn(&KernelModuleContext) -> KResult<()>;
pub type ModuleDestroy = fn(&KernelModuleContext);

#[derive(Clone, Copy)]
pub struct ModuleDescriptor {
    pub name: &'static str,
    pub version: u32,
    pub abi_version: u32,
    pub flags: u32,
    pub init: ModuleInit,
    pub destroy: ModuleDestroy,
    pub dependencies: &'static [&'static str],
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct KernelModuleContext {
    pub module: ModuleId,
    pub api: KernelModuleApi,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct KernelModuleApi {
    pub abi_version: u32,
    pub struct_size: usize,
    pub log: fn(ModuleId, &'static str),
    pub alloc: fn(ModuleId, usize) -> KResult<*mut u8>,
    pub free: fn(ModuleId, *mut u8) -> KResult<()>,
    pub callback_register: fn(ModuleId, ModuleEventKind, ModuleCallback) -> KResult<CallbackToken>,
    pub callback_unregister: fn(CallbackToken) -> KResult<()>,
    pub export_register: fn(ModuleId, &'static str, usize) -> KResult<()>,
    pub time_now: fn() -> Option<TimeValue>,
}

impl KernelModuleApi {
    pub fn validate(self) -> KResult<()> {
        if self.abi_version != MODULE_ABI_VERSION
            || self.struct_size < core::mem::size_of::<KernelModuleApi>()
        {
            return Err(KernelError::ENOEXEC);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleEventKind {
    CpuTick,
    KeyPressed,
    KeyReleased,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallbackToken(u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeValue {
    pub ticks: u64,
    pub frequency_hz: u32,
}

#[derive(Clone, Copy)]
struct ModuleRecord {
    descriptor: ModuleDescriptor,
    state: ModuleState,
    references: usize,
    active_callbacks: usize,
}

#[derive(Clone, Copy)]
struct CallbackRecord {
    token: CallbackToken,
    owner: ModuleId,
    event: ModuleEventKind,
    callback: ModuleCallback,
    active: bool,
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
}

#[derive(Clone, Copy)]
struct Allocation {
    owner: ModuleId,
    ptr: *mut u8,
}

unsafe impl Send for Allocation {}

struct ModuleManager {
    modules: [Option<ModuleRecord>; MAX_MODULES],
    callbacks: [Option<CallbackRecord>; MAX_CALLBACKS],
    dependencies: [Option<Dependency>; MAX_DEPENDENCIES],
    exports: [Option<ExportRecord>; MAX_EXPORTS],
    allocations: [Option<Allocation>; MAX_ALLOCATIONS],
    next_callback: u16,
}

impl ModuleManager {
    const fn new() -> Self {
        Self {
            modules: [None; MAX_MODULES],
            callbacks: [None; MAX_CALLBACKS],
            dependencies: [None; MAX_DEPENDENCIES],
            exports: [None; MAX_EXPORTS],
            allocations: [None; MAX_ALLOCATIONS],
            next_callback: 1,
        }
    }

    fn find(&self, name: &str) -> Option<ModuleId> {
        self.modules.iter().enumerate().find_map(|(index, record)| {
            (record.is_some() && record.unwrap().descriptor.name == name)
                .then_some(ModuleId(index as u16 + 1))
        })
    }

    fn record(&self, id: ModuleId) -> KResult<ModuleRecord> {
        self.modules
            .get(id.index())
            .and_then(|record| *record)
            .ok_or(KernelError::ENOENT)
    }

    fn record_mut(&mut self, id: ModuleId) -> KResult<&mut ModuleRecord> {
        self.modules
            .get_mut(id.index())
            .and_then(Option::as_mut)
            .ok_or(KernelError::ENOENT)
    }
}

static MANAGER: Spinlock<ModuleManager> = Spinlock::new(ModuleManager::new());
static DEFERRED_EVENTS: Spinlock<([Option<ModuleEvent>; MAX_DEFERRED_EVENTS], usize, usize)> =
    Spinlock::new(([None; MAX_DEFERRED_EVENTS], 0, 0));

fn api() -> KernelModuleApi {
    KernelModuleApi {
        abi_version: MODULE_ABI_VERSION,
        struct_size: core::mem::size_of::<KernelModuleApi>(),
        log: module_log,
        alloc: module_alloc,
        free: module_free,
        callback_register,
        callback_unregister: callback_unregister,
        export_register,
        time_now,
    }
}

pub fn register(descriptor: ModuleDescriptor) -> KResult<ModuleId> {
    if descriptor.abi_version != MODULE_ABI_VERSION {
        return Err(KernelError::ENOEXEC);
    }
    let mut manager = MANAGER.lock();
    if manager.find(descriptor.name).is_some() {
        return Err(KernelError::EEXIST);
    }
    let slot = manager
        .modules
        .iter()
        .position(Option::is_none)
        .ok_or(KernelError::ENOSPC)?;
    let id = ModuleId(slot as u16 + 1);
    manager.modules[slot] = Some(ModuleRecord {
        descriptor,
        state: ModuleState::Registered,
        references: 0,
        active_callbacks: 0,
    });
    pr_info!("module registered: {}\n", descriptor.name);
    Ok(id)
}

pub fn find(name: &str) -> Option<ModuleId> {
    MANAGER.lock().find(name)
}

pub fn state(id: ModuleId) -> KResult<ModuleState> {
    MANAGER.lock().record(id).map(|record| record.state)
}

pub fn create(id: ModuleId) -> KResult<()> {
    let descriptor;
    {
        let mut manager = MANAGER.lock();
        descriptor = manager.record(id)?.descriptor;
        if manager.record(id)?.state != ModuleState::Registered {
            return Err(KernelError::EBUSY);
        }
        manager.record_mut(id)?.state = ModuleState::Creating;
        for dependency in descriptor.dependencies {
            if manager.find(dependency).is_none() {
                manager.record_mut(id)?.state = ModuleState::Destroyed;
                return Err(KernelError::ENOENT);
            }
        }
        for dependency in descriptor.dependencies {
            let provider = manager.find(dependency).ok_or(KernelError::ENOENT)?;
            if manager.record(provider)?.state != ModuleState::Active {
                manager.record_mut(id)?.state = ModuleState::Destroyed;
                return Err(KernelError::EBUSY);
            }
            add_dependency_locked(&mut manager, id, provider)?;
        }
        manager.record_mut(id)?.state = ModuleState::Initializing;
    }

    let context = KernelModuleContext { module: id, api: api() };
    if let Err(error) = (descriptor.init)(&context) {
        let _ = cleanup(id, descriptor);
        return Err(error);
    }
    MANAGER.lock().record_mut(id)?.state = ModuleState::Active;
    pr_info!("module initialized: {}\n", descriptor.name);
    Ok(())
}

pub fn destroy(id: ModuleId) -> KResult<()> {
    let descriptor;
    loop {
        let mut manager = MANAGER.lock();
        let record = manager.record_mut(id)?;
        if record.state != ModuleState::Active {
            return Err(KernelError::EBUSY);
        }
        if record.references != 0 {
            return Err(KernelError::EBUSY);
        }
        record.state = ModuleState::Destroying;
        descriptor = record.descriptor;
        break;
    }
    cleanup(id, descriptor)?;
    pr_info!("module destroyed: {}\n", descriptor.name);
    Ok(())
}

fn cleanup(id: ModuleId, descriptor: ModuleDescriptor) -> KResult<()> {
    unregister_all(id);
    loop {
        let active = MANAGER.lock().record(id)?.active_callbacks;
        if active == 0 {
            break;
        }
        core::hint::spin_loop();
    }
    (descriptor.destroy)(&KernelModuleContext { module: id, api: api() });
    let mut owned_allocations = [core::ptr::null_mut(); MAX_ALLOCATIONS];
    let mut allocation_count = 0;
    let mut manager = MANAGER.lock();
    for allocation in &mut manager.allocations {
        if let Some(entry) = *allocation {
            if entry.owner == id {
                owned_allocations[allocation_count] = entry.ptr;
                allocation_count += 1;
                *allocation = None;
            }
        }
    }
    drop(manager);
    for ptr in owned_allocations.iter().take(allocation_count) {
        let _ = paging::kfree(*ptr);
    }
    let mut manager = MANAGER.lock();
    for index in 0..manager.dependencies.len() {
        if let Some(entry) = manager.dependencies[index] {
            if entry.consumer == id {
                if let Ok(provider) = manager.record_mut(entry.provider) {
                    provider.references = provider.references.saturating_sub(1);
                }
                manager.dependencies[index] = None;
            }
        }
    }
    if let Ok(record) = manager.record_mut(id) {
        record.state = ModuleState::Destroyed;
    }
    manager.exports.iter_mut().for_each(|export| {
        if export.map(|entry| entry.owner == id).unwrap_or(false) {
            *export = None;
        }
    });
    Ok(())
}

pub fn unregister_module(id: ModuleId) -> KResult<()> {
    let mut manager = MANAGER.lock();
    let record = manager.record(id)?;
    if record.state != ModuleState::Destroyed || record.references != 0 {
        return Err(KernelError::EBUSY);
    }
    manager.modules[id.index()] = None;
    Ok(())
}

pub fn acquire(id: ModuleId) -> KResult<()> {
    let mut manager = MANAGER.lock();
    let record = manager.record_mut(id)?;
    if record.state != ModuleState::Active {
        return Err(KernelError::EBUSY);
    }
    record.references += 1;
    Ok(())
}

pub fn release(id: ModuleId) -> KResult<()> {
    let mut manager = MANAGER.lock();
    let record = manager.record_mut(id)?;
    if record.references == 0 {
        return Err(KernelError::EINVAL);
    }
    record.references -= 1;
    Ok(())
}

fn add_dependency_locked(manager: &mut ModuleManager, consumer: ModuleId, provider: ModuleId) -> KResult<()> {
    if manager.dependencies.iter().flatten().any(|entry| entry.consumer == consumer && entry.provider == provider) {
        return Ok(());
    }
    let slot = manager.dependencies.iter().position(Option::is_none).ok_or(KernelError::ENOSPC)?;
    manager.dependencies[slot] = Some(Dependency { consumer, provider });
    manager.record_mut(provider)?.references += 1;
    Ok(())
}

pub fn add_dependency(consumer: ModuleId, provider: ModuleId) -> KResult<()> {
    let mut manager = MANAGER.lock();
    if manager.record(provider)?.state != ModuleState::Active {
        return Err(KernelError::EBUSY);
    }
    add_dependency_locked(&mut manager, consumer, provider)
}

fn event_kind(event: ModuleEvent) -> ModuleEventKind {
    match event {
        ModuleEvent::CpuTick { .. } => ModuleEventKind::CpuTick,
        ModuleEvent::KeyPressed(_, _) => ModuleEventKind::KeyPressed,
        ModuleEvent::KeyReleased(_, _) => ModuleEventKind::KeyReleased,
    }
}

pub fn callback_register(owner: ModuleId, event: ModuleEventKind, callback: ModuleCallback) -> KResult<CallbackToken> {
    let mut manager = MANAGER.lock();
    if manager.record(owner)?.state != ModuleState::Initializing
        && manager.record(owner)?.state != ModuleState::Active
    {
        return Err(KernelError::EBUSY);
    }
    let slot = manager.callbacks.iter().position(Option::is_none).ok_or(KernelError::ENOSPC)?;
    let token = CallbackToken(manager.next_callback);
    manager.next_callback = manager.next_callback.wrapping_add(1).max(1);
    manager.callbacks[slot] = Some(CallbackRecord { token, owner, event, callback, active: true });
    pr_debug!("callback registered for module {}\n", owner.0);
    Ok(token)
}

pub fn callback_unregister(token: CallbackToken) -> KResult<()> {
    let mut manager = MANAGER.lock();
    let callback = manager.callbacks.iter_mut().flatten().find(|entry| entry.token == token).ok_or(KernelError::ENOENT)?;
    callback.active = false;
    Ok(())
}

fn unregister_all(owner: ModuleId) {
    let mut manager = MANAGER.lock();
    for callback in &mut manager.callbacks {
        if callback.map(|entry| entry.owner == owner).unwrap_or(false) {
            *callback = None;
        }
    }
}

pub fn dispatch(event: ModuleEvent) {
    let kind = event_kind(event);
    let mut pending: [(ModuleId, ModuleCallback); MAX_CALLBACKS] = [(ModuleId::INVALID, noop); MAX_CALLBACKS];
    let mut count = 0;
    {
        let mut manager = MANAGER.lock();
        for index in 0..manager.callbacks.len() {
            if let Some(callback) = manager.callbacks[index] {
                if callback.active && callback.event == kind && count < MAX_CALLBACKS {
                    if manager.record(callback.owner).map(|record| record.state == ModuleState::Active).unwrap_or(false) {
                    pending[count] = (callback.owner, callback.callback);
                    count += 1;
                    if let Ok(owner) = manager.record_mut(callback.owner) {
                        owner.active_callbacks += 1;
                    }
                }
            }
        }
        }
    }
    for (owner, callback) in pending.iter().take(count) {
        callback(*owner, event, &api());
        let mut manager = MANAGER.lock();
        if let Ok(record) = manager.record_mut(*owner) {
            record.active_callbacks = record.active_callbacks.saturating_sub(1);
        }
    }
}

fn noop(_: ModuleId, _: ModuleEvent, _: &KernelModuleApi) {}

fn module_log(_: ModuleId, message: &'static str) {
    pr_info!("[module] {}\n", message);
}

fn module_alloc(owner: ModuleId, size: usize) -> KResult<*mut u8> {
    {
        let manager = MANAGER.lock();
        if manager.record(owner)?.state != ModuleState::Active
            && manager.record(owner)?.state != ModuleState::Initializing
        {
            return Err(KernelError::EBUSY);
        }
    }
    let ptr = paging::kmalloc(size)?;
    let mut manager = MANAGER.lock();
    if manager.record(owner)?.state != ModuleState::Active
        && manager.record(owner)?.state != ModuleState::Initializing
    {
        drop(manager);
        let _ = paging::kfree(ptr);
        return Err(KernelError::EBUSY);
    }
    let Some(slot) = manager.allocations.iter().position(Option::is_none) else {
        drop(manager);
        let _ = paging::kfree(ptr);
        return Err(KernelError::ENOSPC);
    };
    manager.allocations[slot] = Some(Allocation { owner, ptr });
    Ok(ptr)
}

fn module_free(owner: ModuleId, ptr: *mut u8) -> KResult<()> {
    let mut manager = MANAGER.lock();
    let slot = manager.allocations.iter().position(|entry| entry.map(|allocation| allocation.owner == owner && allocation.ptr == ptr).unwrap_or(false)).ok_or(KernelError::EINVAL)?;
    manager.allocations[slot] = None;
    drop(manager);
    paging::kfree(ptr)
}

fn export_register(owner: ModuleId, name: &'static str, address: usize) -> KResult<()> {
    let mut manager = MANAGER.lock();
    let slot = manager.exports.iter().position(Option::is_none).ok_or(KernelError::ENOSPC)?;
    manager.exports[slot] = Some(ExportRecord { name, owner, address });
    Ok(())
}

fn time_now() -> Option<TimeValue> {
    let manager = MANAGER.lock();
    let export = manager.exports.iter().flatten().find(|entry| entry.name == "time.now")?;
    let function: fn() -> TimeValue = unsafe { core::mem::transmute(export.address) };
    Some(function())
}

pub fn module_time() -> Option<TimeValue> {
    time_now()
}

pub fn dispatch_cpu_tick() {
    let mut queue = DEFERRED_EVENTS.lock();
    let next = (queue.1 + 1) % MAX_DEFERRED_EVENTS;
    if next != queue.2 {
        let head = queue.1;
        queue.0[head] = Some(ModuleEvent::CpuTick {
            cpu_id: unsafe { crate::sched::current_cpu() },
        });
        queue.1 = next;
    }
}

pub fn run_deferred_work() {
    loop {
        let event = {
            let mut queue = DEFERRED_EVENTS.lock();
            if queue.2 == queue.1 {
                None
            } else {
                let tail = queue.2;
                let event = queue.0[tail].take();
                queue.2 = (queue.2 + 1) % MAX_DEFERRED_EVENTS;
                event
            }
        };
        let Some(event) = event else {
            break;
        };
        dispatch(event);
    }
}

pub fn dispatch_key_event(event: KeyEvent, modifiers: Modifiers) {
    dispatch(if event.pressed {
        ModuleEvent::KeyPressed(event, modifiers)
    } else {
        ModuleEvent::KeyReleased(event, modifiers)
    });
}

#[unsafe(link_section = ".init.text")]
pub fn init_modules() {
    let keyboard = match register(keyboard::DESCRIPTOR).and_then(|id| create(id).map(|_| id)) {
        Ok(id) => id,
        Err(error) => {
            pr_err!("keyboard module failed: {:?}\n", error);
            return;
        }
    };
    let time = match register(time::DESCRIPTOR).and_then(|id| create(id).map(|_| id)) {
        Ok(id) => id,
        Err(error) => {
            pr_warn!("time module failed: {:?}\n", error);
            let _ = destroy(keyboard);
            let _ = unregister_module(keyboard);
            return;
        }
    };
    pr_info!("modules active: keyboard={}, time={}\n", keyboard.0, time.0);
}

crate::device_initcall!(init_modules);
