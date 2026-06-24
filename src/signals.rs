use crate::sched::schedule_task;
use crate::pr_warn;
use crate::spin::Spinlock;

pub const MAX_SIGNALS: usize = 32;
pub const MAX_SCHEDULED_SIGNALS: usize = 64;

// TODO: Implement bitmasks and blocking.

#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum Signal {
    SIGHUP = 1,
    SIGINT = 2,
    SIGQUIT = 3,
    SIGILL = 4,
    SIGTRAP = 5,
    SIGABRT = 6,
    SIGBUS = 7,
    SIGFPE = 8,
    SIGKILL = 9,
    SIGUSR1 = 10,
    SIGSEGV = 11,
    SIGUSR2 = 12,
    SIGPIPE = 13,
    SIGALRM = 14,
    SIGTERM = 15,
    SIGSTKFLT = 16,
    SIGCHLD = 17,
    SIGCONT = 18,
    SIGSTOP = 19,
    SIGTSTP = 20,
    SIGTTIN = 21,
    SIGTTOU = 22,
    SIGURG = 23,
    SIGXCPU = 24,
    SIGXFSZ = 25,
    SIGVTALRM = 26,
    SIGPROF = 27,
    SIGWINCH = 28,
    SIGIO = 29,
    SIGPWR = 30,
    SIGSYS = 31,
}

impl Signal {
    pub const fn from_u8(val: u8) -> Option<Self> {
        match val {
            1 => Some(Self::SIGHUP),
            2 => Some(Self::SIGINT),
            3 => Some(Self::SIGQUIT),
            4 => Some(Self::SIGILL),
            5 => Some(Self::SIGTRAP),
            6 => Some(Self::SIGABRT),
            7 => Some(Self::SIGBUS),
            8 => Some(Self::SIGFPE),
            9 => Some(Self::SIGKILL),
            10 => Some(Self::SIGUSR1),
            11 => Some(Self::SIGSEGV),
            12 => Some(Self::SIGUSR2),
            13 => Some(Self::SIGPIPE),
            14 => Some(Self::SIGALRM),
            15 => Some(Self::SIGTERM),
            16 => Some(Self::SIGSTKFLT),
            17 => Some(Self::SIGCHLD),
            18 => Some(Self::SIGCONT),
            19 => Some(Self::SIGSTOP),
            20 => Some(Self::SIGTSTP),
            21 => Some(Self::SIGTTIN),
            22 => Some(Self::SIGTTOU),
            23 => Some(Self::SIGURG),
            24 => Some(Self::SIGXCPU),
            25 => Some(Self::SIGXFSZ),
            26 => Some(Self::SIGVTALRM),
            27 => Some(Self::SIGPROF),
            28 => Some(Self::SIGWINCH),
            29 => Some(Self::SIGIO),
            30 => Some(Self::SIGPWR),
            31 => Some(Self::SIGSYS),
            _ => None,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "SIGHUP" => Some(Self::SIGHUP),
            "SIGINT" => Some(Self::SIGINT),
            "SIGQUIT" => Some(Self::SIGQUIT),
            "SIGILL" => Some(Self::SIGILL),
            "SIGTRAP" => Some(Self::SIGTRAP),
            "SIGABRT" => Some(Self::SIGABRT),
            "SIGBUS" => Some(Self::SIGBUS),
            "SIGFPE" => Some(Self::SIGFPE),
            "SIGKILL" => Some(Self::SIGKILL),
            "SIGUSR1" => Some(Self::SIGUSR1),
            "SIGSEGV" => Some(Self::SIGSEGV),
            "SIGUSR2" => Some(Self::SIGUSR2),
            "SIGPIPE" => Some(Self::SIGPIPE),
            "SIGALRM" => Some(Self::SIGALRM),
            "SIGTERM" => Some(Self::SIGTERM),
            "SIGSTKFLT" => Some(Self::SIGSTKFLT),
            "SIGCHLD" => Some(Self::SIGCHLD),
            "SIGCONT" => Some(Self::SIGCONT),
            "SIGSTOP" => Some(Self::SIGSTOP),
            "SIGTSTP" => Some(Self::SIGTSTP),
            "SIGTTIN" => Some(Self::SIGTTIN),
            "SIGTTOU" => Some(Self::SIGTTOU),
            "SIGURG" => Some(Self::SIGURG),
            "SIGXCPU" => Some(Self::SIGXCPU),
            "SIGXFSZ" => Some(Self::SIGXFSZ),
            "SIGVTALRM" => Some(Self::SIGVTALRM),
            "SIGPROF" => Some(Self::SIGPROF),
            "SIGWINCH" => Some(Self::SIGWINCH),
            "SIGIO" => Some(Self::SIGIO),
            "SIGPWR" => Some(Self::SIGPWR),
            "SIGSYS" => Some(Self::SIGSYS),
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
            sched.pending[i] =
                ScheduledSignal { active: true, signal: sig, ticks_remaining: delay_ms };
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