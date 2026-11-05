//! Module callback management
//! 
//! Lifetime rules:
//!  - Unregistering a slot marks it as revoked so it can't be called again.
//!  - Destroying a module marks all of its slots revoked.
//!  - A slot is freed only when nothing is executing it. If there are, the last
//!    one who finishes the execution is responsible of freeing it.

use core::sync::atomic::{AtomicU32, Ordering};

use super::{
    MANAGER,
    abi::{CallbackToken, ModuleCallbackFn, ModuleEvent, ModuleEventKind, ModuleId},
    context_for,
};
use crate::{
    error::{KResult, KernelError},
    smp::MAX_CPUS,
};

const MAX_CALLBACKS: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SlotState {
    Active,
    Revoked,
}

#[derive(Clone, Copy)]
struct CallbackSlot {
    owner: ModuleId,
    kind: ModuleEventKind,
    function: ModuleCallbackFn,
    data: usize,
    state: SlotState,
    in_flight: u32,
}

pub(super) struct CallbackTable {
    slots: [Option<CallbackSlot>; MAX_CALLBACKS],
    generations: [u16; MAX_CALLBACKS],
}

// To prevent unnecessary dispatch when no one is requesting it
static REGISTERED: [AtomicU32; ModuleEventKind::COUNT] =
    [const { AtomicU32::new(0) }; ModuleEventKind::COUNT];

// To prevent CPUs from deadlocking themselves upon create/destroy
static DEPTH: [AtomicU32; MAX_CPUS] = [const { AtomicU32::new(0) }; MAX_CPUS];

impl CallbackTable {
    pub(super) const fn new() -> Self {
        Self {
            slots: [None; MAX_CALLBACKS],
            generations: [0; MAX_CALLBACKS],
        }
    }

    pub(super) fn len(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    fn token(&self, index: usize) -> CallbackToken {
        CallbackToken(((self.generations[index] as u32) << 16) | (index as u32 + 1))
    }

    fn index_of(&self, token: CallbackToken) -> Option<usize> {
        let index = (token.0 & 0xFFFF).checked_sub(1)? as usize;
        (index < MAX_CALLBACKS
            && self.generations[index] == (token.0 >> 16) as u16
            && self.slots[index].is_some())
        .then_some(index)
    }

    fn free(&mut self, index: usize) {
        if let Some(slot) = self.slots[index].take() {
            self.generations[index] = self.generations[index].wrapping_add(1);
            REGISTERED[slot.kind as usize].fetch_sub(1, Ordering::Relaxed);
        }
    }

    fn revoke(&mut self, index: usize) {
        let Some(slot) = self.slots[index].as_mut() else {
            return;
        };
        slot.state = SlotState::Revoked;
        if slot.in_flight == 0 {
            self.free(index);
        }
    }
}

fn current_cpu() -> usize {
    unsafe { crate::sched::current_cpu() as usize }
}

pub(super) fn in_callback() -> bool {
    DEPTH[current_cpu()].load(Ordering::Relaxed) != 0
}

pub(super) fn register(
    owner: ModuleId,
    kind: ModuleEventKind,
    function: ModuleCallbackFn,
    data: usize,
) -> KResult<CallbackToken> {
    let mut manager = MANAGER.lock();
    if !manager.is_live(owner) {
        return Err(KernelError::Perm);
    }
    let table = &mut manager.callbacks;
    let index = table
        .slots
        .iter()
        .position(Option::is_none)
        .ok_or(KernelError::NoSpc)?;
    table.slots[index] = Some(CallbackSlot {
        owner,
        kind,
        function,
        data,
        state: SlotState::Active,
        in_flight: 0,
    });
    REGISTERED[kind as usize].fetch_add(1, Ordering::Relaxed);
    Ok(table.token(index))
}

pub(super) fn unregister(owner: ModuleId, token: CallbackToken) -> KResult<()> {
    let mut manager = MANAGER.lock();
    let table = &mut manager.callbacks;
    let index = table.index_of(token).ok_or(KernelError::NoEnt)?;
    let slot = table.slots[index].ok_or(KernelError::NoEnt)?;
    if slot.owner != owner {
        return Err(KernelError::Perm);
    }
    if slot.state == SlotState::Revoked {
        return Err(KernelError::NoEnt);
    }
    table.revoke(index);
    Ok(())
}

pub(super) fn revoke_all(owner: ModuleId) {
    {
        let mut manager = MANAGER.lock();
        for index in 0..MAX_CALLBACKS {
            if manager.callbacks.slots[index].is_some_and(|slot| slot.owner == owner) {
                manager.callbacks.revoke(index);
            }
        }
    }
    while MANAGER
        .lock()
        .callbacks
        .slots
        .iter()
        .flatten()
        .any(|slot| slot.owner == owner)
    {
        core::hint::spin_loop();
    }
}

/// 1. Lock
/// 2. `in_flight++`
/// 3. Unlock
/// 4. execute callback
/// 5. Lock
/// 6. `in_flight--`
/// 7. Unlock
pub(crate) fn dispatch(event: ModuleEvent) {
    let kind = event.kind();
    if REGISTERED[kind as usize].load(Ordering::Relaxed) == 0 {
        return;
    }

    let mut pending: [Option<(usize, ModuleId, ModuleCallbackFn, usize)>; MAX_CALLBACKS] =
        [None; MAX_CALLBACKS];
    let mut count = 0;
    {
        let mut manager = MANAGER.lock();
        for index in 0..MAX_CALLBACKS {
            let Some(slot) = manager.callbacks.slots[index] else {
                continue;
            };
            if slot.kind != kind
                || slot.state != SlotState::Active
                || manager.state(slot.owner) != Some(super::ModuleState::Active)
            {
                continue;
            }
            if let Some(slot) = manager.callbacks.slots[index].as_mut() {
                slot.in_flight += 1;
            }
            pending[count] = Some((index, slot.owner, slot.function, slot.data));
            count += 1;
        }
    }

    let depth = &DEPTH[current_cpu()];
    depth.fetch_add(1, Ordering::Relaxed);
    for (index, owner, function, data) in pending[..count].iter().flatten().copied() {
        function(&context_for(owner), &event, data);

        let mut manager = MANAGER.lock();
        let table = &mut manager.callbacks;
        if let Some(slot) = table.slots[index].as_mut() {
            slot.in_flight -= 1;
            if slot.in_flight == 0 && slot.state == SlotState::Revoked {
                table.free(index);
            }
        }
    }
    depth.fetch_sub(1, Ordering::Relaxed);
}
