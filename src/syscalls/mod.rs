mod syscalls;

mod open;
mod exit;
mod fork;
mod mem;
mod read_write;
mod signal;
mod socket;
mod sys;
mod time;

pub use syscalls::init_syscalls;
