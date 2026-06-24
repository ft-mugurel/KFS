use core::fmt;
use core::sync::atomic::{AtomicU8, Ordering};

use crate::startup_config;
use crate::vga::text_mod;

pub static LOG_LEVEL: AtomicU8 = AtomicU8::new(KernelLogLevel::Info as u8);

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

pub fn printk_level_to_screen(
    screen_index: usize,
    level: KernelLogLevel,
    args: &fmt::Arguments<'_>,
) {
    if !is_enabled(level) {
        return;
    }
    text_mod::print_str_on(screen_index, level_tag(level));
    text_mod::print_fmt_on(screen_index, args);
}

pub fn printk_level_to_default(level: KernelLogLevel, args: &fmt::Arguments<'_>) {
    if !is_enabled(level) {
        return;
    }
    printk_level_to_screen(startup_config::logging::DEFAULT_LOG_SCREEN, level, args);
}

#[allow(dead_code)]
pub fn printk_to_screen(screen_index: usize, args: &fmt::Arguments<'_>) {
    printk_level_to_screen(screen_index, KernelLogLevel::Info, args);
}

pub fn printk_to_debug(args: &fmt::Arguments<'_>) {
    let screen_index = if is_enabled(KernelLogLevel::Debug) {
        startup_config::logging::DEFAULT_LOG_SCREEN
    } else {
        startup_config::logging::DEFAULT_DEBUG_LOG_SCREEN
    };
    text_mod::print_str_on(screen_index, level_tag(KernelLogLevel::Debug));
    text_mod::print_fmt_on(screen_index, args);
}
