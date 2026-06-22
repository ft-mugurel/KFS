pub mod syscalls;
pub use syscalls::init_syscalls;

mod mem;
mod read_write;
mod close;
mod exit;
mod fork;
mod sys;
mod signal;
mod time;
mod socket;
