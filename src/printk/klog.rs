use crate::locks::Spinlock;
use crate::printk::KernelLogLevel;
use core::fmt::{self, Write};

pub const KLOG_BUFFER_SIZE: usize = 65536;

pub struct KLogBuffer {
    data: [u8; KLOG_BUFFER_SIZE],
    head: usize,
}

impl KLogBuffer {
    const fn new() -> Self {
        Self { data: [0; KLOG_BUFFER_SIZE], head: 0 }
    }

    fn push_byte(&mut self, b: u8) {
        let idx = self.head % KLOG_BUFFER_SIZE;
        self.data[idx] = b;
        self.head = self.head.wrapping_add(1);
    }
}

static KLOG: Spinlock<KLogBuffer> = Spinlock::new(KLogBuffer::new());

fn level_tag_str(level: KernelLogLevel) -> &'static str {
    match level {
        KernelLogLevel::Emerg => "[EMG] ",
        KernelLogLevel::Alert => "[ALR] ",
        KernelLogLevel::Crit => "[CRT] ",
        KernelLogLevel::Err => "[ERR] ",
        KernelLogLevel::Warning => "[WRN] ",
        KernelLogLevel::Notice => "[NTC] ",
        KernelLogLevel::Info => "[INF] ",
        KernelLogLevel::Debug => "[DBG] ",
    }
}

struct AnsiFilterWriter<'a> {
    buf: &'a mut KLogBuffer,
    in_escape: bool,
}

impl<'a> Write for AnsiFilterWriter<'a> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &b in s.as_bytes() {
            if self.in_escape {
                if b == b'm' || (b >= b'A' && b <= b'Z') || (b >= b'a' && b <= b'z') {
                    self.in_escape = false;
                }
            } else if b == 0x1B {
                self.in_escape = true;
            } else {
                self.buf.push_byte(b);
            }
        }
        Ok(())
    }
}

pub fn record_log(level: KernelLogLevel, pid: u32, ticks: usize, args: &fmt::Arguments<'_>) {
    let mut klog = KLOG.lock();

    let hz = crate::startup_config::power::CONFIG_HZ as usize;
    let sec = ticks / hz;
    let millis = ticks % hz;

    let mut writer = AnsiFilterWriter { buf: &mut klog, in_escape: false };

    let _ = write!(
        writer,
        "[{:>5}.{:03}] {}({}) ",
        sec,
        millis,
        level_tag_str(level),
        pid
    );
    let _ = write!(writer, "{}", args);
}

pub fn head() -> usize {
    let klog = KLOG.lock();
    klog.head
}

pub fn clear() {
    let mut klog = KLOG.lock();
    klog.head = 0;
    klog.data = [0; KLOG_BUFFER_SIZE];
}

pub fn read_range(start: usize, dest: &mut [u8]) -> usize {
    let klog = KLOG.lock();
    let current_head = klog.head;

    if start >= current_head || dest.is_empty() {
        return 0;
    }

    let oldest_valid = current_head.saturating_sub(KLOG_BUFFER_SIZE);
    let actual_start = if start < oldest_valid {
        oldest_valid
    } else {
        start
    };

    let available = current_head - actual_start;
    let to_read = available.min(dest.len());

    for i in 0..to_read {
        let pos = actual_start + i;
        dest[i] = klog.data[pos % KLOG_BUFFER_SIZE];
    }

    to_read
}

pub fn read_kmsg(dest: &mut [u8], offset: u32) -> usize {
    let klog = KLOG.lock();
    let current_head = klog.head;
    let oldest_valid = current_head.saturating_sub(KLOG_BUFFER_SIZE);
    let total_available = current_head - oldest_valid;

    let offset = offset as usize;
    if offset >= total_available || dest.is_empty() {
        return 0;
    }

    let actual_start = oldest_valid + offset;
    let available = current_head - actual_start;
    let to_read = available.min(dest.len());

    for i in 0..to_read {
        let pos = actual_start + i;
        dest[i] = klog.data[pos % KLOG_BUFFER_SIZE];
    }

    to_read
}
