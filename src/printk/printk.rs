use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use crate::locks::Spinlock;
use crate::vga::text_mod;

pub static LOG_LEVEL: AtomicU8 = AtomicU8::new(KernelLogLevel::Info as u8);
pub static DIRECT_SCREEN_OUTPUT: AtomicBool = AtomicBool::new(true);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum KernelLogLevel {
    Emerg = 0,
    Alert = 1,
    Crit = 2,
    Err = 3,
    Warning = 4,
    Notice = 5,
    Info = 6,
    Debug = 7,
}

const fn level_tag(level: KernelLogLevel) -> &'static str {
    match level {
        KernelLogLevel::Emerg => "[\x1B\x0F;\x14mEMG\x1Bm] ",
        KernelLogLevel::Alert => "[\x1B\x0F;\x16mALR\x1Bm] ",
        KernelLogLevel::Crit => "[\x1B\x0F;\x1CmCRT\x1Bm] ",
        KernelLogLevel::Err => "[\x1B\x04mERR\x1Bm] ",
        KernelLogLevel::Warning => "[\x1B\x0EmWRN\x1Bm] ",
        KernelLogLevel::Notice => "[\x1B\x09mNTC\x1Bm] ",
        KernelLogLevel::Info => "[\x1B\x07mINF\x1Bm] ",
        KernelLogLevel::Debug => "[\x1B\x08mDBG\x1Bm] ",
    }
}

pub fn set_log_level(level: KernelLogLevel) {
    LOG_LEVEL.store(level as u8, Ordering::Relaxed);
}

fn is_enabled(level: KernelLogLevel) -> bool {
    level as u8 <= LOG_LEVEL.load(Ordering::Relaxed)
}

#[allow(dead_code)]
pub fn is_direct_screen_output_enabled() -> bool {
    DIRECT_SCREEN_OUTPUT.load(Ordering::Acquire)
}

#[allow(dead_code)]
pub fn set_direct_screen_output(enabled: bool) {
    DIRECT_SCREEN_OUTPUT.store(enabled, Ordering::Release);
}

pub fn handoff_to_userspace() {
    DIRECT_SCREEN_OUTPUT.store(false, Ordering::Release);
}

static LOCKS: [Spinlock<()>; 8] = [
    Spinlock::new(()),
    Spinlock::new(()),
    Spinlock::new(()),
    Spinlock::new(()),
    Spinlock::new(()),
    Spinlock::new(()),
    Spinlock::new(()),
    Spinlock::new(()),
];

pub fn printk_emit(
    screen_index: usize,
    level: KernelLogLevel,
    args: &fmt::Arguments<'_>,
    force_screen: bool,
) {
    if !is_enabled(level) {
        return;
    }

    let pid = unsafe { crate::sched::current_pid() };
    let ticks = crate::interrupts::timer::get_ticks();
    super::klog::record_log(level, pid, ticks, args);

    let to_screen =
        force_screen || is_direct_screen_output_enabled() || level == KernelLogLevel::Emerg;

    if to_screen {
        let lock = &LOCKS[screen_index % LOCKS.len()];
        let _guard = lock.lock();
        text_mod::print_str_on(screen_index, level_tag(level));
        text_mod::print_fmt_on(
            screen_index,
            &format_args!("({}) ", pid),
        );
        text_mod::print_fmt_on(screen_index, args);
    } else {
        crate::serial::write_printk_to_serial(level_tag(level), pid, args);
    }
}

pub fn printk_level_to_default(level: KernelLogLevel, args: &fmt::Arguments<'_>) {
    let screen = if level == KernelLogLevel::Emerg {
        text_mod::active_screen_index()
    } else {
        0
    };
    printk_emit(screen, level, args, false);
}

#[allow(dead_code)]
pub fn printk_level_to_screen(
    screen_index: usize,
    level: KernelLogLevel,
    args: &fmt::Arguments<'_>,
) {
    printk_emit(screen_index, level, args, false);
}

#[allow(dead_code)]
pub fn printk_to_screen(screen_index: usize, args: &fmt::Arguments<'_>) {
    printk_emit(screen_index, KernelLogLevel::Info, args, false);
}

pub fn printk_to_debug(args: &fmt::Arguments<'_>) {
    if is_enabled(KernelLogLevel::Debug) {
        printk_emit(0, KernelLogLevel::Debug, args, false);
    }
}
