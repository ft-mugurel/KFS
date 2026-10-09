use core::fmt::Write;

use crate::{
    drivers, fs, interrupts, paging, smp,
};

use super::BufferWriter;

pub fn generate_version(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);
    let _ = writeln!(writer, "kernel: KFS");
    let _ = writeln!(writer, "release: 0.6.0");
    let _ = writeln!(writer, "arch: i686 (32-bit x86 Protected Mode)");
    let _ = writeln!(writer, "compiler: rustc (nightly 2026-09-22)");
    let _ = writeln!(
        writer,
        "subsystems: SMP, ACPI, Ext2, Paging, DAC, PBKDF2-HMAC-SHA256, Procfs"
    );
    writer.written()
}

pub fn generate_uptime(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);
    let ticks = interrupts::timer::get_ticks();
    let hz = 100;
    let sec = ticks / hz;
    let centisec = ((ticks % hz) * 100) / hz;
    let _ = writeln!(writer, "uptime_seconds: {}.{:02}", sec, centisec);
    let _ = writeln!(writer, "uptime_ticks: {}", ticks);
    let _ = writeln!(writer, "timer_frequency_hz: {}", hz);
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
    let _ = writeln!(writer, "VMemTotal: {} KiB", (vstats.total_bytes as usize) / 1024);
    let _ = writeln!(writer, "VMemFree: {} KiB", (vstats.free_bytes as usize) / 1024);
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
        if f_edx & (1 << 0) != 0 { let _ = write!(writer, " fpu"); }
        if f_edx & (1 << 1) != 0 { let _ = write!(writer, " vme"); }
        if f_edx & (1 << 2) != 0 { let _ = write!(writer, " de"); }
        if f_edx & (1 << 3) != 0 { let _ = write!(writer, " pse"); }
        if f_edx & (1 << 4) != 0 { let _ = write!(writer, " tsc"); }
        if f_edx & (1 << 5) != 0 { let _ = write!(writer, " msr"); }
        if f_edx & (1 << 6) != 0 { let _ = write!(writer, " pae"); }
        if f_edx & (1 << 8) != 0 { let _ = write!(writer, " cx8"); }
        if f_edx & (1 << 9) != 0 { let _ = write!(writer, " apic"); }
        if f_edx & (1 << 11) != 0 { let _ = write!(writer, " sep"); }
        if f_edx & (1 << 13) != 0 { let _ = write!(writer, " pge"); }
        if f_edx & (1 << 15) != 0 { let _ = write!(writer, " cmov"); }
        if f_edx & (1 << 19) != 0 { let _ = write!(writer, " clflush"); }
        if f_edx & (1 << 23) != 0 { let _ = write!(writer, " mmx"); }
        if f_edx & (1 << 24) != 0 { let _ = write!(writer, " fxsr"); }
        if f_edx & (1 << 25) != 0 { let _ = write!(writer, " sse"); }
        if f_edx & (1 << 26) != 0 { let _ = write!(writer, " sse2"); }
        if f_ecx & (1 << 0) != 0 { let _ = write!(writer, " sse3"); }
        let _ = writeln!(writer);
        let _ = writeln!(writer);
    }

    writer.written()
}

pub fn generate_stat(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);
    let mut active_count = 0usize;
    let mut running_count = 0usize;
    let mut sleeping_count = 0usize;

    for process in crate::sched::PROCESS_TABLE.lock().iter().flatten() {
        active_count += 1;
        match process.state {
            crate::sched::ProcessState::Running => running_count += 1,
            crate::sched::ProcessState::Sleeping | crate::sched::ProcessState::Waiting => {
                sleeping_count += 1
            }
            _ => {}
        }
    }

    let ticks = interrupts::timer::get_ticks();
    let _ = writeln!(writer, "cpu_cores_total: {}", smp::MAX_CPUS);
    let _ = writeln!(writer, "cpu_cores_online: {}", smp::cpu::online_count());
    let _ = writeln!(writer, "processes_total: {}", active_count);
    let _ = writeln!(writer, "processes_running: {}", running_count);
    let _ = writeln!(writer, "processes_sleeping: {}", sleeping_count);
    let _ = writeln!(writer, "timer_ticks: {}", ticks);
    writer.written()
}

pub fn generate_partitions(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);
    let _ = writeln!(
        writer,
        "{:<3} {:<10} {:<10} {:<7} {:<10} {}",
        "ID", "NAME", "KIND", "PARENT", "START_LBA", "SECTORS"
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
        let kind = if device.partition {
            "partition"
        } else {
            "disk"
        };
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

pub fn generate_mounts(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);
    let _ = writeln!(
        writer,
        "{:<12} {:<10} {:<10} {}",
        "device", "mountpoint", "filesystem", "options"
    );

    fs::for_each_mount(|mount| {
        let (dev_name, mount_point, fs_type, opts) = match mount.private_data {
            fs::MountPrivate::Ext2(_) => ("/dev/hda1", "/", "ext2", "rw,relatime"),
            fs::MountPrivate::Procfs => ("procfs", "/proc", "procfs", "ro,nosuid,nodev"),
            fs::MountPrivate::Sysfs => ("sysfs", "/sys", "sysfs", "ro,nosuid,nodev"),
            fs::MountPrivate::Raw => ("raw", "/mnt", "raw", "rw"),
        };
        let _ = writeln!(
            writer,
            "{:<12} {:<10} {:<10} {}",
            dev_name, mount_point, fs_type, opts
        );
    });

    writer.written()
}

pub fn generate_interrupts(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);
    let _ = writeln!(
        writer,
        "{:<5} {:<10} {:<16} {}",
        "IRQ", "CONTROLLER", "DEVICE", "STATUS"
    );
    let _ = writeln!(
        writer,
        "{:<5} {:<10} {:<16} {}",
        "0", "PIC/APIC", "timer", "enabled"
    );
    let _ = writeln!(
        writer,
        "{:<5} {:<10} {:<16} {}",
        "1", "PIC", "keyboard", "enabled"
    );
    let _ = writeln!(
        writer,
        "{:<5} {:<10} {:<16} {}",
        "14", "PCI/IDE", "ata_primary", "enabled"
    );
    let _ = writeln!(
        writer,
        "{:<5} {:<10} {:<16} {}",
        "15", "PCI/IDE", "ata_secondary", "enabled"
    );
    writer.written()
}

pub fn generate_cmdline(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);
    let _ = writeln!(writer, "BOOT_IMAGE=/boot/kernel.bin quiet root=/dev/hda1");
    writer.written()
}

pub fn generate_devices(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);
    let _ = writeln!(writer, "Character devices:");
    let _ = writeln!(writer, "  1 tty (virtual screens 1-6)");
    let _ = writeln!(writer, "  5 /dev/tty");
    let _ = writeln!(writer);
    let _ = writeln!(writer, "Block devices:");
    let _ = writeln!(writer, "  3 ide (ATA IDE Controller)");
    writer.written()
}

pub fn generate_loadavg(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);
    let mut running = 0usize;
    let mut total = 0usize;

    for p in crate::sched::PROCESS_TABLE.lock().iter().flatten() {
        total += 1;
        if p.state == crate::sched::ProcessState::Running {
            running += 1;
        }
    }

    let _ = writeln!(writer, "0.00 0.00 0.00 {}/{}", running, total);
    writer.written()
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
