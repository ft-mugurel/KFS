use crate::interrupts::task_queue::schedule_task;
use crate::pr_warn;
use crate::spin::Spinlock;

pub const MAX_SIGNALS: usize = 32;
pub const MAX_SCHEDULED_SIGNALS: usize = 64;

#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum Signal {
    SIGHUP = 1,
    SIGINT = 2,
    SIGQUIT = 3,
    SIGILL = 4,
    SIGTRAP = 5,
    SIGABRT = 6,
    SIGKILL = 9,
    SIGSEGV = 11,
    SIGALRM = 14,
    SIGTERM = 15,
    SIGUSR1 = 10,
    SIGUSR2 = 12,
}

impl Signal {
    fn from_u8(val: u8) -> Option<Self> {
        match val {
            1 => Some(Self::SIGHUP),
            2 => Some(Self::SIGINT),
            3 => Some(Self::SIGQUIT),
            4 => Some(Self::SIGILL),
            5 => Some(Self::SIGTRAP),
            6 => Some(Self::SIGABRT),
            9 => Some(Self::SIGKILL),
            11 => Some(Self::SIGSEGV),
            14 => Some(Self::SIGALRM),
            15 => Some(Self::SIGTERM),
            10 => Some(Self::SIGUSR1),
            12 => Some(Self::SIGUSR2),
            _ => None,
        }
    }
}

pub type SignalHandler = fn();

#[derive(Copy, Clone)]
struct ScheduledSignal {
    active: bool,
    signal: Signal,
    ticks_remaining: u64,
}

struct SignalRegistry {
    handlers: [Option<SignalHandler>; MAX_SIGNALS],
}

struct SignalScheduler {
    pending: [ScheduledSignal; MAX_SCHEDULED_SIGNALS],
}

static REGISTRY: Spinlock<SignalRegistry> =
    Spinlock::new(SignalRegistry { handlers: [None; MAX_SIGNALS] });

static SCHEDULER: Spinlock<SignalScheduler> = Spinlock::new(SignalScheduler {
    pending: [ScheduledSignal {
        active: false,
        signal: Signal::SIGHUP,
        ticks_remaining: 0,
    }; MAX_SCHEDULED_SIGNALS],
});

pub fn register_signal_handler(sig: Signal, handler: SignalHandler) {
    let mut reg = REGISTRY.lock();
    reg.handlers[sig as usize] = Some(handler);
}

pub fn send_signal(sig: Signal) {
    let reg = REGISTRY.lock();
    if let Some(handler) = reg.handlers[sig as usize] {
        schedule_task(handler);
    } else {
        pr_warn!("Unhandled signal: {}\n", sig as u8);
    }
}

pub fn schedule_signal(sig: Signal, delay_ms: u64) {
    let mut sched = SCHEDULER.lock();
    for i in 0..MAX_SCHEDULED_SIGNALS {
        if !sched.pending[i].active {
            sched.pending[i] = ScheduledSignal {
                active: true,
                signal: sig,
                ticks_remaining: delay_ms,
            };
            return;
        }
    }
    pr_warn!("Signal scheduler queue full!\n");
}

pub fn process_scheduled_signals() {
    let mut sched = SCHEDULER.lock();
    
    for i in 0..MAX_SCHEDULED_SIGNALS {
        if sched.pending[i].active {
            if sched.pending[i].ticks_remaining > 0 {
                sched.pending[i].ticks_remaining -= 1;
            } 
            
            if sched.pending[i].ticks_remaining == 0 {
                // Time is up, send the signal to the task queue
                send_signal(sched.pending[i].signal);
                sched.pending[i].active = false;
            }
        }
    }
}