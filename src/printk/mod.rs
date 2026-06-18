pub mod printk;

pub use printk::{set_log_level, KernelLogLevel};

#[macro_export]
macro_rules! printk {
	($($arg:tt)*) => {
		$crate::printk::printk::printk_level_to_default($crate::printk::printk::KernelLogLevel::Info, &core::format_args!($($arg)*))
	};
}

#[macro_export]
macro_rules! printk_level {
	($level:expr, $($arg:tt)*) => {
		$crate::printk::printk::printk_level_to_default($level, &core::format_args!($($arg)*))
	};
}

#[macro_export]
macro_rules! printk_level_on {
	($screen_index:expr, $level:expr, $($arg:tt)*) => {
		$crate::printk::printk::printk_level_to_screen($screen_index, $level, &core::format_args!($($arg)*))
	};
}

#[macro_export]
macro_rules! printk_on {
	($screen_index:expr, $($arg:tt)*) => {
		$crate::printk::printk::printk_level_to_screen($screen_index, $crate::printk::printk::KernelLogLevel::Info, &core::format_args!($($arg)*))
	};
}

#[macro_export]
macro_rules! pr_debug {
	($($arg:tt)*) => {
		$crate::printk::printk::printk_to_debug(&core::format_args!($($arg)*))
	};
}

#[macro_export]
macro_rules! pr_info {
	($($arg:tt)*) => {
		$crate::printk_level!($crate::printk::printk::KernelLogLevel::Info, $($arg)*)
	};
}

#[macro_export]
macro_rules! pr_notice {
	($($arg:tt)*) => {
		$crate::printk_level!($crate::printk::printk::KernelLogLevel::Notice, $($arg)*)
	};
}

#[macro_export]
macro_rules! pr_warn {
	($($arg:tt)*) => {
		$crate::printk_level!($crate::printk::printk::KernelLogLevel::Warning, $($arg)*)
	};
}

#[macro_export]
macro_rules! pr_err {
	($($arg:tt)*) => {
		$crate::printk_level!($crate::printk::printk::KernelLogLevel::Err, $($arg)*)
	};
}

#[macro_export]
macro_rules! pr_crit {
	($($arg:tt)*) => {
		$crate::printk_level!($crate::printk::printk::KernelLogLevel::Crit, $($arg)*)
	};
}

#[macro_export]
macro_rules! pr_alert {
	($($arg:tt)*) => {
		$crate::printk_level!($crate::printk::printk::KernelLogLevel::Alert, $($arg)*)
	};
}

#[macro_export]
macro_rules! pr_emerg {
	($($arg:tt)*) => {
		$crate::printk_level!($crate::printk::printk::KernelLogLevel::Emerg, $($arg)*)
	};
}
