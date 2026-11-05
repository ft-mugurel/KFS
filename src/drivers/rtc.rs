use core::sync::atomic::{AtomicU32, Ordering};

use crate::{
    locks::{Spinlock, SpinlockGuard},
    x86::{inb, outb},
};

const CMOS_ADDRESS: u16 = 0x70;
const CMOS_DATA: u16 = 0x71;
const REG_SECONDS: u8 = 0x00;
const REG_MINUTES: u8 = 0x02;
const REG_HOURS: u8 = 0x04;
const REG_DAY: u8 = 0x07;
const REG_MONTH: u8 = 0x08;
const REG_YEAR: u8 = 0x09;
const REG_STATUS_A: u8 = 0x0A;
const REG_STATUS_B: u8 = 0x0B;
const STATUS_A_UPDATING: u8 = 1 << 7;
const STATUS_B_24H: u8 = 1 << 1;
const STATUS_B_BINARY: u8 = 1 << 2;
const HOUR_PM: u8 = 1 << 7;

struct Cmos;
static CMOS_LOCK: Spinlock<Cmos> = Spinlock::new(Cmos);
static BOOT_TIME: AtomicU32 = AtomicU32::new(0);

#[derive(PartialEq, Eq, Clone, Copy)]
struct RawTime([u8; 6]);

fn read_register(_held: &SpinlockGuard<'_, Cmos>, reg: u8) -> u8 {
    // SAFETY: 0x70/0x71 are the CMOS index/data ports, and `_held` proves
    // `CMOS_LOCK` is held, so no other CPU can change the index between the
    // write and the read. Register numbers are below 0x80, so bit 7 (NMI
    // disable) stays clear.
    unsafe {
        outb(CMOS_ADDRESS, reg);
        inb(CMOS_DATA)
    }
}

fn read_raw(_held: &SpinlockGuard<'_, Cmos>) -> RawTime {
    while read_register(_held, REG_STATUS_A) & STATUS_A_UPDATING != 0 {}
    RawTime([
        read_register(_held, REG_SECONDS),
        read_register(_held, REG_MINUTES),
        read_register(_held, REG_HOURS),
        read_register(_held, REG_DAY),
        read_register(_held, REG_MONTH),
        read_register(_held, REG_YEAR),
    ])
}

fn bcd(value: u8) -> u8 {
    (value & 0x0F) + (value >> 4) * 10
}

/// Seconds since the Unix epoch according to the RTC
/// (can't work before 2000)
pub fn now_epoch_seconds() -> u32 {
    let (raw, status_b) = {
        let guard = CMOS_LOCK.lock();
        let mut previous = read_raw(&guard);
        loop {
            let current = read_raw(&guard);
            if current == previous {
                break;
            }
            previous = current;
        }
        (previous.0, read_register(&guard, REG_STATUS_B))
    };

    let [mut sec, mut min, mut hour, mut day, mut month, mut year] = raw;
    let pm = hour & HOUR_PM != 0;
    hour &= !HOUR_PM;
    if status_b & STATUS_B_BINARY == 0 {
        sec = bcd(sec);
        min = bcd(min);
        hour = bcd(hour);
        day = bcd(day);
        month = bcd(month);
        year = bcd(year);
    }
    if status_b & STATUS_B_24H == 0 {
        hour %= 12;
        if pm {
            hour += 12;
        }
    }

    let days = days_from_civil(2000 + year as u32, month as u32, day as u32);
    days * 86_400 + hour as u32 * 3600 + min as u32 * 60 + sec as u32
}

fn days_from_civil(year: u32, month: u32, day: u32) -> u32 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year / 400;
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub fn boot_time(uptime_secs: u32) -> u32 {
    let cached = BOOT_TIME.load(Ordering::Relaxed);
    if cached != 0 {
        return cached;
    }
    let boot = now_epoch_seconds().saturating_sub(uptime_secs);
    BOOT_TIME.store(boot, Ordering::Relaxed);
    boot
}
