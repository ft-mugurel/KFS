use crate::printk::KernelLogLevel;
use crate::vga::text_mod::Color;

pub fn parse_log_level(name: &str) -> Option<KernelLogLevel> {
    match name {
        "emerg" => Some(KernelLogLevel::Emerg),
        "alert" => Some(KernelLogLevel::Alert),
        "crit" => Some(KernelLogLevel::Crit),
        "err" => Some(KernelLogLevel::Err),
        "warn" | "warning" => Some(KernelLogLevel::Warning),
        "notice" => Some(KernelLogLevel::Notice),
        "info" => Some(KernelLogLevel::Info),
        "debug" => Some(KernelLogLevel::Debug),
        _ => None,
    }
}

pub fn parse_color(name: &str) -> Option<Color> {
    match name {
        "white" => Some(Color::White),
        "gray" | "lightgray" => Some(Color::LightGray),
        "red" => Some(Color::LightRed),
        "green" => Some(Color::LightGreen),
        "blue" => Some(Color::LightBlue),
        "yellow" => Some(Color::Yellow),
        "cyan" => Some(Color::LightCyan),
        "magenta" => Some(Color::Pink),
        _ => None,
    }
}

pub fn strip_hex_prefix(input: &str) -> Option<&str> {
    let input = input.trim();
    input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
}

pub fn parse_u32(input: &str) -> Option<u32> {
    let input = input.trim();
    if let Some(hex) = strip_hex_prefix(input) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        input.parse::<u32>().ok()
    }
}

pub fn parse_usize(input: &str) -> Option<usize> {
    let input = input.trim();
    if let Some(hex) = strip_hex_prefix(input) {
        usize::from_str_radix(hex, 16).ok()
    } else {
        input.parse::<usize>().ok()
    }
}
