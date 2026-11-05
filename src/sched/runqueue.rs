//! Ready queue and timed sleepers, kept in sync by `set_state()`.
//!
//! `schedule()` takes the next task from the front of the ready queue instead
//! of scanning the process table, and wakes only the sleepers whose tick has
//! passed (they are kept sorted by wakeup tick). Idle tasks (PIDs below
//! MAX_CPUS) are never queued: a CPU falls back to its own idle task when the
//! ready queue is empty.
//!
//! RUN_QUEUE is a leaf lock. It is taken with or without PROCESS_TABLE held,
//! and never takes another lock itself.

use core::sync::atomic::{AtomicUsize, Ordering};

use super::{MAX_PROCESSES, ProcessState};
use crate::{locks::Spinlock, smp::MAX_CPUS};

struct RunQueue {
    ready: [u32; MAX_PROCESSES],
    head: usize,
    len: usize,
    /// Whether a PID is in `ready`, so pushes and removals never duplicate
    /// or scan for an entry that isn't there.
    queued: [bool; MAX_PROCESSES],
    /// (wakeup tick, pid), sorted by tick, earliest first.
    sleepers: [(u64, u32); MAX_PROCESSES],
    sleeper_count: usize,
}

impl RunQueue {
    const fn new() -> Self {
        Self {
            ready: [0; MAX_PROCESSES],
            head: 0,
            len: 0,
            queued: [false; MAX_PROCESSES],
            sleepers: [(0, 0); MAX_PROCESSES],
            sleeper_count: 0,
        }
    }

    fn push_ready(&mut self, pid: usize) {
        if self.queued[pid] {
            return;
        }
        // Cannot overflow: each PID is queued at most once.
        self.ready[(self.head + self.len) % MAX_PROCESSES] = pid as u32;
        self.len += 1;
        self.queued[pid] = true;
    }

    fn pop_ready(&mut self) -> Option<usize> {
        if self.len == 0 {
            return None;
        }
        let pid = self.ready[self.head] as usize;
        self.head = (self.head + 1) % MAX_PROCESSES;
        self.len -= 1;
        self.queued[pid] = false;
        Some(pid)
    }

    /// Only reached when a task leaves Ready without being dispatched,
    /// which is rare, but still..
    fn remove_ready(&mut self, pid: usize) {
        if !self.queued[pid] {
            return;
        }
        let mut kept = 0;
        for i in 0..self.len {
            let entry = self.ready[(self.head + i) % MAX_PROCESSES];
            if entry as usize != pid {
                self.ready[(self.head + kept) % MAX_PROCESSES] = entry;
                kept += 1;
            }
        }
        self.len = kept;
        self.queued[pid] = false;
    }

    fn insert_sleeper(&mut self, pid: usize, wakeup_tick: u64) {
        self.remove_sleeper(pid);
        let count = self.sleeper_count;
        let at = self.sleepers[..count]
            .iter()
            .position(|&(tick, _)| tick > wakeup_tick)
            .unwrap_or(count);
        self.sleepers.copy_within(at..count, at + 1);
        self.sleepers[at] = (wakeup_tick, pid as u32);
        self.sleeper_count += 1;
    }

    fn remove_sleeper(&mut self, pid: usize) {
        let count = self.sleeper_count;
        if let Some(at) = self.sleepers[..count]
            .iter()
            .position(|&(_, sleeper)| sleeper as usize == pid)
        {
            self.sleepers.copy_within(at + 1..count, at);
            self.sleeper_count -= 1;
        }
    }

    fn publish(&self) {
        RUNNABLE_TASKS.store(self.len, Ordering::SeqCst);
        let next = if self.sleeper_count == 0 {
            usize::MAX
        } else {
            self.sleepers[0].0 as usize
        };
        NEXT_WAKEUP_TICK.store(next, Ordering::SeqCst);
    }
}

static RUN_QUEUE: Spinlock<RunQueue> = Spinlock::new(RunQueue::new());
static RUNNABLE_TASKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_WAKEUP_TICK: AtomicUsize = AtomicUsize::new(usize::MAX);

fn is_queueable(pid: usize) -> bool {
    (MAX_CPUS..MAX_PROCESSES).contains(&pid)
}

pub(super) fn state_changed(pid: usize, old: ProcessState, new: ProcessState, wakeup_tick: u64) {
    if !is_queueable(pid) {
        return;
    }
    let mut queue = RUN_QUEUE.lock();
    match old {
        ProcessState::Ready => queue.remove_ready(pid),
        ProcessState::Sleeping => queue.remove_sleeper(pid),
        _ => {}
    }
    match new {
        ProcessState::Ready => queue.push_ready(pid),
        ProcessState::Sleeping => queue.insert_sleeper(pid, wakeup_tick),
        _ => {}
    }
    queue.publish();
}

pub(super) fn pop_ready() -> Option<usize> {
    let mut queue = RUN_QUEUE.lock();
    let pid = queue.pop_ready();
    queue.publish();
    pid
}

/// Moves every sleeper whose tick has passed into `due` and returns how many.
/// The caller still has to set each one Ready.
pub(super) fn take_due_sleepers(now: u64, due: &mut [usize; MAX_PROCESSES]) -> usize {
    let mut queue = RUN_QUEUE.lock();
    let count = queue.sleepers[..queue.sleeper_count]
        .iter()
        .take_while(|&&(tick, _)| tick <= now)
        .count();
    for (slot, &(_, pid)) in due.iter_mut().zip(&queue.sleepers[..count]) {
        *slot = pid as usize;
    }
    let remaining = queue.sleeper_count;
    queue.sleepers.copy_within(count..remaining, 0);
    queue.sleeper_count -= count;
    queue.publish();
    count
}

pub(super) fn runnable_count() -> usize {
    RUNNABLE_TASKS.load(Ordering::SeqCst)
}

pub(super) fn wakeup_may_be_due(current_tick: u64) -> bool {
    (current_tick as usize) >= NEXT_WAKEUP_TICK.load(Ordering::SeqCst)
}

/// Exercises the queue logic on private RunQueue instances, plus the one
/// global path that must stay a no-op: idle PIDs entering Ready.
pub(crate) fn selftest(check: &mut dyn FnMut(&'static str, bool)) {
    let a = MAX_CPUS;
    let b = MAX_CPUS + 1;
    let c = MAX_CPUS + 2;

    let mut queue = RunQueue::new();
    queue.push_ready(a);
    queue.push_ready(b);
    queue.push_ready(a);
    check("runqueue ignores duplicate push", queue.len == 2);
    check(
        "runqueue is FIFO",
        queue.pop_ready() == Some(a) && queue.pop_ready() == Some(b) && queue.pop_ready().is_none(),
    );

    queue.push_ready(a);
    queue.push_ready(b);
    queue.push_ready(c);
    queue.remove_ready(b);
    check(
        "runqueue remove keeps order",
        queue.pop_ready() == Some(a) && queue.pop_ready() == Some(c) && queue.len == 0,
    );
    queue.push_ready(b);
    check(
        "runqueue requeue after remove",
        queue.pop_ready() == Some(b),
    );

    let mut queue = RunQueue::new();
    queue.insert_sleeper(a, 30);
    queue.insert_sleeper(b, 10);
    queue.insert_sleeper(c, 20);
    queue.insert_sleeper(a, 5); // re-sleeping replaces the old entry
    let order: [u32; 3] = [
        queue.sleepers[0].1,
        queue.sleepers[1].1,
        queue.sleepers[2].1,
    ];
    check(
        "runqueue sleepers sorted by tick",
        queue.sleeper_count == 3 && order == [a as u32, b as u32, c as u32],
    );
    queue.remove_sleeper(b);
    check(
        "runqueue sleeper removal",
        queue.sleeper_count == 2 && queue.sleepers[1].1 == c as u32,
    );

    // Idle tasks belong to one CPU each, queueing one would let another CPU
    // dispatch it.
    let before = runnable_count();
    for idle in 0..MAX_CPUS {
        state_changed(idle, ProcessState::Sleeping, ProcessState::Ready, 0);
    }
    check(
        "runqueue never queues idle tasks",
        runnable_count() == before,
    );
}
