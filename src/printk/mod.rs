pub mod file_logger;
pub mod klog;
pub mod printk;

#[allow(unused_imports)]
pub use printk::{
    handoff_to_userspace, is_direct_screen_output_enabled, set_direct_screen_output, set_log_level,
    KernelLogLevel,
};

#[macro_export]
macro_rules! pr_debug {
	($($arg:tt)*) => {
		$crate::printk::printk::printk_to_debug(&core::format_args!($($arg)*))
	};
}

#[macro_export]
macro_rules! pr_info {
	($($arg:tt)*) => {
		$crate::printk::printk::printk_level_to_default($crate::printk::printk::KernelLogLevel::Info, &core::format_args!($($arg)*))
	};
}

#[macro_export]
macro_rules! pr_notice {
	($($arg:tt)*) => {
		$crate::printk::printk::printk_level_to_default($crate::printk::printk::KernelLogLevel::Notice, &core::format_args!($($arg)*))
	};
}

#[macro_export]
macro_rules! pr_warn {
	($($arg:tt)*) => {
		$crate::printk::printk::printk_level_to_default($crate::printk::printk::KernelLogLevel::Warning, &core::format_args!($($arg)*))
	};
}

#[macro_export]
macro_rules! pr_err {
	($($arg:tt)*) => {
		$crate::printk::printk::printk_level_to_default($crate::printk::printk::KernelLogLevel::Err, &core::format_args!($($arg)*))
	};
}

#[macro_export]
macro_rules! pr_crit {
	($($arg:tt)*) => {
		$crate::printk::printk::printk_level_to_default($crate::printk::printk::KernelLogLevel::Crit, &core::format_args!($($arg)*))
	};
}

#[macro_export]
macro_rules! pr_alert {
	($($arg:tt)*) => {
		$crate::printk::printk::printk_level_to_default($crate::printk::printk::KernelLogLevel::Alert, &core::format_args!($($arg)*))
	};
}

#[macro_export]
macro_rules! pr_emerg {
	($($arg:tt)*) => {
		$crate::printk::printk::printk_level_to_default($crate::printk::printk::KernelLogLevel::Emerg, &core::format_args!($($arg)*))
	};
}
