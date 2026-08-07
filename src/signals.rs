use crate::{pr_err, sched};

// TODO: Implement bitmasks and blocking.

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
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

pub unsafe fn register_signal_handler(sig: Signal, handler: u32) {
    let Some(task) = &mut sched::current().as_mut() else {
        pr_err!("Failed to register signal handler: no current task\n");
        return;
    };
    let task_sigs = &mut task.signals;
    task_sigs.handlers[sig as usize] = handler;
}

pub unsafe fn send_signal(sig: Signal) {
    let Some(task) = sched::current().as_mut() else {
        pr_err!("Failed to send signal: no current task\n");
        return;
    };
    let task_sigs = &mut task.signals;
    let next_tail = (task_sigs.tail + 1) % task_sigs.pending.len();
    if next_tail != task_sigs.head {
        task_sigs.pending[task_sigs.tail] = sig as u8;
        task_sigs.tail = next_tail;
    }
}

pub unsafe fn process_scheduled_signals() {
    let Some(task) = sched::current().as_mut() else {
        pr_err!("Failed to process scheduled signals: no current task\n");
        return;
    };
    let task_sigs = &mut task.signals;

    while task_sigs.head != task_sigs.tail {
        let sig_num = task_sigs.pending[task_sigs.head];
        let handler_addr = task_sigs.handlers[sig_num as usize];

        if handler_addr != 0 {
            let handler: extern "C" fn(u32) = core::mem::transmute(handler_addr);
            handler(sig_num as u32);
        }
        task_sigs.head = (task_sigs.head + 1) % task_sigs.pending.len();
    }
}
