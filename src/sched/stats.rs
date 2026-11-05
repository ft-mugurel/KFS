//! Scheduler accounting behind /proc/stat, /proc/uptime and /proc/loadavg.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::smp::MAX_CPUS;

#[derive(Clone, Copy, Default)]
pub(crate) struct CpuTimes {
    pub user: u32,
    pub system: u32,
    pub idle: u32,
}

struct CpuCounters {
    user: AtomicU32,
    system: AtomicU32,
    idle: AtomicU32,
    busy: AtomicBool,
}

impl CpuCounters {
    const fn new() -> Self {
        Self {
            user: AtomicU32::new(0),
            system: AtomicU32::new(0),
            idle: AtomicU32::new(0),
            busy: AtomicBool::new(false),
        }
    }
}

static CPU_TIMES: [CpuCounters; MAX_CPUS] = [const { CpuCounters::new() }; MAX_CPUS];
static CONTEXT_SWITCHES: AtomicU32 = AtomicU32::new(0);
static PROCESSES_CREATED: AtomicU32 = AtomicU32::new(0);
static LAST_PID: AtomicU32 = AtomicU32::new(0);

pub(crate) const FSHIFT: u32 = 11;
pub(crate) const FIXED_1: u32 = 1 << FSHIFT;
const LOAD_FREQ_SECS: usize = 5;
const EXP: [u32; 3] = [1884, 2014, 2037];
static LOAD_AVR: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];

pub(crate) unsafe fn account_tick(cpu_id: u32, frame_esp: u32) {
    let Some(cpu) = CPU_TIMES.get(cpu_id as usize) else {
        return;
    };
    let idle = unsafe { super::current_pid() } < MAX_CPUS as u32;
    cpu.busy.store(!idle, Ordering::Relaxed);
    let counter = if idle {
        &cpu.idle
    } else {
        let frame = unsafe { &*(frame_esp as *const super::ContextFrame) };
        if frame.cs & 0x3 == 3 { &cpu.user } else { &cpu.system }
    };
    counter.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn global_tick(ticks: usize, hz: u32) {
    let period = LOAD_FREQ_SECS * hz.max(1) as usize;
    if ticks == 0 || !ticks.is_multiple_of(period) {
        return;
    }
    let active = running_now() as u32 * FIXED_1;
    for (avg, exp) in LOAD_AVR.iter().zip(EXP) {
        let load = avg.load(Ordering::Relaxed);
        let mut next = load * exp + active * (FIXED_1 - exp);
        if active >= load {
            next += FIXED_1 - 1;
        }
        avg.store(next >> FSHIFT, Ordering::Relaxed);
    }
}

pub(crate) fn running_now() -> usize {
    let busy = CPU_TIMES
        .iter()
        .filter(|cpu| cpu.busy.load(Ordering::Relaxed))
        .count();
    busy + super::runnable_task_count()
}

pub(crate) fn note_context_switch() {
    CONTEXT_SWITCHES.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn note_process_created(pid: u32) {
    PROCESSES_CREATED.fetch_add(1, Ordering::Relaxed);
    LAST_PID.store(pid, Ordering::Relaxed);
}

pub(crate) fn cpu_times(cpu_id: usize) -> CpuTimes {
    CPU_TIMES
        .get(cpu_id)
        .map_or(CpuTimes::default(), |cpu| CpuTimes {
            user: cpu.user.load(Ordering::Relaxed),
            system: cpu.system.load(Ordering::Relaxed),
            idle: cpu.idle.load(Ordering::Relaxed),
        })
}

pub(crate) fn context_switches() -> u32 {
    CONTEXT_SWITCHES.load(Ordering::Relaxed)
}

pub(crate) fn processes_created() -> u32 {
    PROCESSES_CREATED.load(Ordering::Relaxed)
}

pub(crate) fn last_pid() -> u32 {
    LAST_PID.load(Ordering::Relaxed)
}

/// 1, 5 and 15 minute load averages, fixed point with FSHIFT fraction bits.
pub(crate) fn load_averages() -> [u32; 3] {
    [
        LOAD_AVR[0].load(Ordering::Relaxed),
        LOAD_AVR[1].load(Ordering::Relaxed),
        LOAD_AVR[2].load(Ordering::Relaxed),
    ]
}
