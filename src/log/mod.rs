pub mod file_logger;
pub mod klog;
pub mod printk;

use core::sync::atomic::{AtomicBool, AtomicU8};

pub static LOG_LEVEL: AtomicU8 = AtomicU8::new(KernelLogLevel::Info as u8);
pub static DIRECT_SCREEN_OUTPUT: AtomicBool = AtomicBool::new(true);

pub use printk::{
    KernelLogLevel, handoff_to_userspace, is_direct_screen_output_enabled,
    set_direct_screen_output, set_log_level,
};

#[macro_export]
macro_rules! pr_debug {
	($($arg:tt)*) => {
		$crate::log::printk::printk_to_debug(&core::format_args!($($arg)*))
	};
}

#[macro_export]
macro_rules! pr_info {
	($($arg:tt)*) => {
		$crate::log::printk::printk_level_to_default($crate::log::printk::KernelLogLevel::Info, &core::format_args!($($arg)*))
	};
}

#[macro_export]
macro_rules! pr_notice {
	($($arg:tt)*) => {
		$crate::log::printk::printk_level_to_default($crate::log::printk::KernelLogLevel::Notice, &core::format_args!($($arg)*))
	};
}

#[macro_export]
macro_rules! pr_warn {
	($($arg:tt)*) => {
		$crate::log::printk::printk_level_to_default($crate::log::printk::KernelLogLevel::Warning, &core::format_args!($($arg)*))
	};
}

#[macro_export]
macro_rules! pr_err {
	($($arg:tt)*) => {
		$crate::log::printk::printk_level_to_default($crate::log::printk::KernelLogLevel::Err, &core::format_args!($($arg)*))
	};
}

#[macro_export]
macro_rules! pr_crit {
	($($arg:tt)*) => {
		$crate::log::printk::printk_level_to_default($crate::log::printk::KernelLogLevel::Crit, &core::format_args!($($arg)*))
	};
}

#[macro_export]
macro_rules! pr_alert {
	($($arg:tt)*) => {
		$crate::log::printk::printk_level_to_default($crate::log::printk::KernelLogLevel::Alert, &core::format_args!($($arg)*))
	};
}

#[macro_export]
macro_rules! pr_emerg {
	($($arg:tt)*) => {
		$crate::log::printk::printk_level_to_default($crate::log::printk::KernelLogLevel::Emerg, &core::format_args!($($arg)*))
	};
}
