use core::fmt::Write;

use crate::{
    drivers,
    error::{KResult, KernelError},
    fs, interrupts, paging, sched, smp,
};

use super::BufferWriter;

pub fn generate_version(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);
    let _ = writeln!(
        writer,
        "KFS version {} ({}) ({} build) #1 SMP i686",
        env!("CARGO_PKG_VERSION"),
        env!("KFS_RUSTC_VERSION"),
        env!("KFS_BUILD_PROFILE"),
    );
    writer.written()
}

fn uptime_ms() -> u64 {
    match crate::modules::time_now() {
        Ok(now) => now.uptime_ms,
        Err(_) => {
            let ticks = interrupts::timer::get_ticks() as u64;
            let hz = interrupts::timer::tick_frequency_hz() as u64;
            ticks * 1000 / hz.max(1)
        }
    }
}

fn ticks_to_clock(ticks: u32) -> u64 {
    let hz = interrupts::timer::tick_frequency_hz().max(1) as u64;
    ticks as u64 * 100 / hz
}

fn online_cpus() -> impl Iterator<Item = usize> {
    (0..smp::MAX_CPUS).filter(|&cpu| smp::cpu::is_online(cpu))
}

pub fn generate_uptime(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);
    let uptime_cs = uptime_ms() / 10;
    let idle_cs: u64 = online_cpus()
        .map(|cpu| ticks_to_clock(sched::stats::cpu_times(cpu).idle))
        .sum();
    let _ = writeln!(
        writer,
        "{}.{:02} {}.{:02}",
        uptime_cs / 100,
        uptime_cs % 100,
        idle_cs / 100,
        idle_cs % 100
    );
    writer.written()
}

pub fn generate_meminfo(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);

    let total_pages = paging::total_physical_pages();
    let free_pages = paging::free_physical_pages();
    let used_pages = total_pages.saturating_sub(free_pages);
    let page_size = paging::physical_page_size();
    let total_phys_kib = total_pages.saturating_mul(page_size) / 1024;
    let free_phys_kib = free_pages.saturating_mul(page_size) / 1024;
    let used_phys_kib = used_pages.saturating_mul(page_size) / 1024;

    let vstats = paging::vmem_debug_stats();
    let hstats = paging::kernel_heap_debug_stats();

    let _ = writeln!(writer, "PageSize: {} B", page_size);
    let _ = writeln!(writer, "PhysTotalPages: {}", total_pages);
    let _ = writeln!(writer, "PhysFreePages: {}", free_pages);
    let _ = writeln!(writer, "PhysUsedPages: {}", used_pages);
    let _ = writeln!(writer, "PhysTotal: {} KiB", total_phys_kib);
    let _ = writeln!(writer, "PhysFree: {} KiB", free_phys_kib);
    let _ = writeln!(writer, "PhysUsed: {} KiB", used_phys_kib);
    let _ = writeln!(writer);
    let _ = writeln!(writer, "KernelHeapReady: {}", hstats.ready);
    let _ = writeln!(writer, "KernelHeapChunks: {}", hstats.chunk_count);
    let _ = writeln!(writer, "KernelHeapTotal: {} KiB", hstats.chunk_bytes / 1024);
    let _ = writeln!(writer, "KernelHeapFree: {} KiB", hstats.free_bytes / 1024);
    let _ = writeln!(writer, "KernelHeapUsedBlocks: {}", hstats.used_block_count);
    let _ = writeln!(writer, "KernelHeapFreeBlocks: {}", hstats.free_block_count);
    let _ = writeln!(writer);
    let _ = writeln!(writer, "VMemRangeStart: {:#010X}", vstats.range_start);
    let _ = writeln!(writer, "VMemRangeEnd: {:#010X}", vstats.range_end);
    let _ = writeln!(
        writer,
        "VMemTotal: {} KiB",
        (vstats.total_bytes as usize) / 1024
    );
    let _ = writeln!(
        writer,
        "VMemFree: {} KiB",
        (vstats.free_bytes as usize) / 1024
    );
    let _ = writeln!(writer, "VMemAllocations: {}", vstats.alloc_count);
    let _ = writeln!(writer, "VMemFreeRanges: {}", vstats.free_ranges);

    writer.written()
}

pub fn generate_cpuinfo(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);

    // Query CPUID vendor string
    let (_, ebx, ecx, edx) = cpuid(0);
    let mut vendor = [0u8; 12];
    vendor[0..4].copy_from_slice(&ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&edx.to_le_bytes());
    vendor[8..12].copy_from_slice(&ecx.to_le_bytes());
    let vendor_str = core::str::from_utf8(&vendor).unwrap_or("Unknown");

    // Query CPUID features
    let (_, _, f_ecx, f_edx) = cpuid(1);

    for cpu_id in 0..smp::MAX_CPUS {
        let is_online = smp::cpu::is_online(cpu_id);
        if !is_online && cpu_id > 0 && cpu_id >= smp::cpu::cpu_count() {
            continue;
        }

        let apic_id = smp::cpu::apic_id_of(cpu_id);
        let role = if cpu_id == 0 { "BSP" } else { "AP" };
        let status = if is_online { "online" } else { "offline" };

        let _ = writeln!(writer, "processor: {}", cpu_id);
        let _ = writeln!(writer, "apic_id: {}", apic_id);
        let _ = writeln!(writer, "status: {}", status);
        let _ = writeln!(writer, "role: {}", role);
        let _ = writeln!(writer, "vendor_id: {}", vendor_str);

        // Feature flags string
        let _ = write!(writer, "features:");
        if f_edx & (1 << 0) != 0 {
            let _ = write!(writer, " fpu");
        }
        if f_edx & (1 << 1) != 0 {
            let _ = write!(writer, " vme");
        }
        if f_edx & (1 << 2) != 0 {
            let _ = write!(writer, " de");
        }
        if f_edx & (1 << 3) != 0 {
            let _ = write!(writer, " pse");
        }
        if f_edx & (1 << 4) != 0 {
            let _ = write!(writer, " tsc");
        }
        if f_edx & (1 << 5) != 0 {
            let _ = write!(writer, " msr");
        }
        if f_edx & (1 << 6) != 0 {
            let _ = write!(writer, " pae");
        }
        if f_edx & (1 << 8) != 0 {
            let _ = write!(writer, " cx8");
        }
        if f_edx & (1 << 9) != 0 {
            let _ = write!(writer, " apic");
        }
        if f_edx & (1 << 11) != 0 {
            let _ = write!(writer, " sep");
        }
        if f_edx & (1 << 13) != 0 {
            let _ = write!(writer, " pge");
        }
        if f_edx & (1 << 15) != 0 {
            let _ = write!(writer, " cmov");
        }
        if f_edx & (1 << 19) != 0 {
            let _ = write!(writer, " clflush");
        }
        if f_edx & (1 << 23) != 0 {
            let _ = write!(writer, " mmx");
        }
        if f_edx & (1 << 24) != 0 {
            let _ = write!(writer, " fxsr");
        }
        if f_edx & (1 << 25) != 0 {
            let _ = write!(writer, " sse");
        }
        if f_edx & (1 << 26) != 0 {
            let _ = write!(writer, " sse2");
        }
        if f_ecx & (1 << 0) != 0 {
            let _ = write!(writer, " sse3");
        }
        let _ = writeln!(writer);
        let _ = writeln!(writer);
    }

    writer.written()
}

pub fn generate_stat(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);

    let write_cpu =
        |writer: &mut BufferWriter, label: &dyn core::fmt::Display, t: sched::stats::CpuTimes| {
            // user nice system idle iowait irq softirq steal guest guest_nice
            let _ = writeln!(
                writer,
                "{} {} 0 {} {} 0 0 0 0 0 0",
                label,
                ticks_to_clock(t.user),
                ticks_to_clock(t.system),
                ticks_to_clock(t.idle)
            );
        };

    let mut total = sched::stats::CpuTimes::default();
    for cpu in online_cpus() {
        let t = sched::stats::cpu_times(cpu);
        total.user = total.user.wrapping_add(t.user);
        total.system = total.system.wrapping_add(t.system);
        total.idle = total.idle.wrapping_add(t.idle);
    }
    write_cpu(&mut writer, &"cpu ", total);
    for cpu in online_cpus() {
        write_cpu(
            &mut writer,
            &format_args!("cpu{}", cpu),
            sched::stats::cpu_times(cpu),
        );
    }

    let uptime_secs = (uptime_ms() / 1000) as u32;

    let _ = writeln!(writer, "intr {}", interrupts::stats::total());
    let _ = writeln!(writer, "ctxt {}", sched::stats::context_switches());
    let _ = writeln!(writer, "btime {}", drivers::rtc::boot_time(uptime_secs));
    let _ = writeln!(writer, "processes {}", sched::stats::processes_created());
    let _ = writeln!(writer, "procs_running {}", sched::stats::running_now());
    let _ = writeln!(writer, "procs_blocked 0"); /* we don't support this for now */
    writer.written()
}

pub fn generate_partitions(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);
    let _ = writeln!(
        writer,
        "{:<3} {:<10} {:<10} {:<7} {:<10} SECTORS",
        "ID", "NAME", "KIND", "PARENT", "START_LBA"
    );

    for device_id in 0..drivers::MAX_BLOCK_DEVICES {
        let Some(device) = drivers::device_at(device_id) else {
            continue;
        };
        let name_len = device
            .name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(device.name.len());
        let name = core::str::from_utf8(&device.name[..name_len]).unwrap_or("?");
        let kind = if device.partition { "partition" } else { "disk" };
        let parent_str = if device.partition {
            let parent_id = device.parent;
            let _ = writeln!(
                writer,
                "{:<3} {:<10} {:<10} {:<7} {:<10} {}",
                device_id, name, kind, parent_id, device.start_lba, device.sector_count
            );
            continue;
        } else {
            "-"
        };
        let _ = writeln!(
            writer,
            "{:<3} {:<10} {:<10} {:<7} {:<10} {}",
            device_id, name, kind, parent_str, device.start_lba, device.sector_count
        );
    }

    writer.written()
}

struct DeviceName([u8; 32]);

impl core::fmt::Display for DeviceName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let len = self.0.iter().position(|&b| b == 0).unwrap_or(self.0.len());
        f.write_str(core::str::from_utf8(&self.0[..len]).unwrap_or("?"))
    }
}

fn device_name(device_id: drivers::BlockDeviceId) -> Option<DeviceName> {
    drivers::device_at(device_id).map(|device| DeviceName(device.name))
}

pub fn generate_mounts(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);
    fs::for_each_mount(|mount| {
        let (fs_type, opts) = match mount.private_data {
            fs::MountPrivate::Ext2(_) => ("ext2", "rw,relatime"),
            fs::MountPrivate::Procfs => ("proc", "rw,nosuid,nodev,noexec"),
            fs::MountPrivate::Sysfs => ("sysfs", "rw,nosuid,nodev,noexec"),
        };
        let _ = match mount.private_data {
            fs::MountPrivate::Ext2(_) => match device_name(mount.device) {
                Some(name) => write!(writer, "/dev/{}", name),
                None => write!(writer, "/dev/unknown"),
            },
            _ => write!(writer, "{}", fs_type),
        };
        let _ = writeln!(writer, " {} {} {} 0 0", mount.mount_point, fs_type, opts);
    });
    writer.written()
}

pub fn generate_interrupts(buf: &mut [u8]) -> usize {
    use interrupts::stats::{IrqLabel, IrqSource};

    let mut writer = BufferWriter::new(buf);
    let _ = write!(writer, "    ");
    for cpu in online_cpus() {
        // Width doesn't apply to format_args!, so pad by hand
        let digits = if cpu >= 10 { 2 } else { 1 };
        let _ = write!(writer, "{:>width$}CPU{}", "", cpu, width = 11 - 3 - digits);
    }
    let _ = writeln!(writer);

    for source in IrqSource::ALL {
        if !interrupts::stats::is_installed(source) {
            continue;
        }
        let _ = match source.label() {
            IrqLabel::Line(line) => write!(writer, "{:>3}:", line),
            IrqLabel::Tag(tag) => write!(writer, "{:>3}:", tag),
        };
        for cpu in online_cpus() {
            let _ = write!(writer, " {:>10}", interrupts::stats::count_on(source, cpu));
        }
        let _ = writeln!(writer, "   {}", source.description());
    }
    writer.written()
}

pub fn generate_cmdline(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);
    let _ = writeln!(writer, "{}", paging::boot_cmdline());
    writer.written()
}

pub fn generate_devices(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);
    let _ = writeln!(writer, "Character devices:");
    for (major, name) in fs::dev::CHAR_DRIVERS {
        let _ = writeln!(writer, "{:>3} {}", major, name);
    }
    let _ = writeln!(writer);
    let _ = writeln!(writer, "Block devices:");
    let mut disk_index = 0;
    for device_id in 0..drivers::MAX_BLOCK_DEVICES {
        let Some(device) = drivers::device_at(device_id) else {
            continue;
        };
        if device.partition {
            continue;
        }
        if let (Some(major), Some(name)) =
            (fs::dev::IDE_MAJORS.get(disk_index), device_name(device_id))
        {
            let _ = writeln!(writer, "{:>3} {}", major, name);
        }
        disk_index += 1;
    }
    writer.written()
}

pub fn generate_loadavg(buf: &mut [u8]) -> usize {
    use sched::stats::{FIXED_1, FSHIFT};

    let mut writer = BufferWriter::new(buf);
    for avg in sched::stats::load_averages() {
        let avg = avg + FIXED_1 / 200;
        let whole = avg >> FSHIFT;
        let frac = ((avg & (FIXED_1 - 1)) * 100) >> FSHIFT;
        let _ = write!(writer, "{}.{:02} ", whole, frac);
    }

    let tasks = sched::PROCESS_TABLE
        .lock()
        .iter()
        .flatten()
        .filter(|process| process.pid >= smp::MAX_CPUS as u32)
        .count();
    let _ = writeln!(
        writer,
        "{}/{} {}",
        sched::stats::running_now(),
        tasks,
        sched::stats::last_pid()
    );
    writer.written()
}

pub fn generate_printk(buf: &mut [u8]) -> usize {
    let mut writer: BufferWriter<'_> = BufferWriter::new(buf);
    let level = crate::log::LOG_LEVEL.load(core::sync::atomic::Ordering::Relaxed);
    let _ = writeln!(writer, "{}", level);
    writer.written()
}

pub fn write_printk(buffer: &[u8]) -> KResult<()> {
    use crate::log::printk::KernelLogLevel;

    let value = core::str::from_utf8(buffer).map_err(|_| KernelError::Inval)?;
    let first = value.split_whitespace().next().ok_or(KernelError::Inval)?;
    let level = match first.parse::<u8>().map_err(|_| KernelError::Inval)? {
        0 => KernelLogLevel::Emerg,
        1 => KernelLogLevel::Alert,
        2 => KernelLogLevel::Crit,
        3 => KernelLogLevel::Err,
        4 => KernelLogLevel::Warning,
        5 => KernelLogLevel::Notice,
        6 => KernelLogLevel::Info,
        7 => KernelLogLevel::Debug,
        _ => return Err(KernelError::Inval),
    };
    crate::log::printk::set_log_level(level);
    Ok(())
}

#[inline(always)]
fn cpuid(leaf: u32) -> (u32, u32, u32, u32) {
    let eax: u32;
    let ebx: u32;
    let ecx: u32;
    let edx: u32;
    unsafe {
        core::arch::asm!(
            "cpuid",
            inout("eax") leaf => eax,
            out("ebx") ebx,
            out("ecx") ecx,
            out("edx") edx,
            options(nomem, nostack, preserves_flags)
        );
    }
    (eax, ebx, ecx, edx)
}
