use crate::{pr_debug, pr_err, pr_info, pr_warn};

use crate::smp::{self, MAX_CPUS};

#[repr(C, packed)]
struct RsdpV1 {
    signature: [u8; 8], // "RSD PTR "
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_addr: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug)]
struct SdtHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug)]
struct MadtHeader {
    header: SdtHeader,
    local_apic_addr: u32,
    flags: u32,
    // entries follow, variable length TLV records
}

pub struct CpuInfo {
    pub apic_id: u8,
    pub bsp: bool,  // apic_id == BSP's own, determined separately via CPUID/MSR
    pub flags: u32, // ACPI flags for the CPU
}

pub struct AcpiInfo {
    pub local_apic_addr: u32,
    pub ioapic_addr: u32,
    pub cpus: [Option<CpuInfo>; MAX_CPUS],
    pub cpu_count: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AcpiError {
    RsdpNotFound,
    // RsdtNotFound,
    MadtNotFound,
    InvalidChecksum,
}

#[unsafe(no_mangle)]
#[unsafe(link_section = ".init.text")]
pub unsafe fn init() -> Option<AcpiInfo> {
    let Ok(rsdp_addr) = find_rsdp() else {
        pr_err!("Failed to find RSDP");
        return None;
    };
    let Ok(info) = parse(rsdp_addr) else {
        pr_err!("Failed to parse ACPI tables");
        return None;
    };
    pr_debug!(
        "Found {} CPU(s), LAPIC @ {:#x}, IOAPIC @ {:#x}\n",
        info.cpu_count,
        info.local_apic_addr,
        info.ioapic_addr
    );
    info.cpus.iter().enumerate().for_each(|(i, cpu)| {
        if let Some(cpu_info) = cpu {
            pr_debug!(
                "CPU {}: APIC ID {}, Flags {:#x}\n",
                i,
                cpu_info.apic_id,
                cpu_info.flags
            );
        }
    });
    Some(info)
}

#[unsafe(no_mangle)]
#[unsafe(link_section = ".init.text")]
unsafe fn find_rsdp() -> Result<u32, AcpiError> {
    let ebda_seg = *(0x40E as *const u16);
    let ebda_addr = (ebda_seg as u32) << 4;
    if let Ok(addr) = scan_for_signature(ebda_addr, ebda_addr + 1024) {
        return Ok(addr);
    } else {
        pr_info!("RSDP not found in EBDA, scanning 0xE0000-0x100000\n");
    }
    scan_for_signature(0xE0000, 0x100000)
}

#[unsafe(no_mangle)]
#[unsafe(link_section = ".init.text")]
unsafe fn scan_for_signature(start: u32, end: u32) -> Result<u32, AcpiError> {
    let mut addr = start;
    while addr < end {
        let sig = core::slice::from_raw_parts(addr as *const u8, 8);
        if sig == b"RSD PTR " {
            if checksum_ok(addr, size_of::<RsdpV1>()) {
                return Ok(addr);
            } else {
                pr_warn!("Invalid checksum for RSDP at {:#x}\n", addr);
                return Err(AcpiError::InvalidChecksum);
            }
        }
        addr += 16;
    }
    Err(AcpiError::RsdpNotFound)
}

#[unsafe(no_mangle)]
#[unsafe(link_section = ".init.text")]
unsafe fn checksum_ok(addr: u32, len: usize) -> bool {
    let bytes = core::slice::from_raw_parts(addr as *const u8, len);
    let mut sum: u16 = 0;
    for i in 0..len {
        sum += bytes[i] as u16;
    }
    sum & 0xFF == 0
}

#[unsafe(no_mangle)]
#[unsafe(link_section = ".init.text")]
unsafe fn parse(rsdp_addr: u32) -> Result<AcpiInfo, AcpiError> {
    let rsdp = rsdp_addr as *const RsdpV1;
    let rsdt = (*rsdp).rsdt_addr as *const SdtHeader;

    let entries_ptr = ((*rsdp).rsdt_addr + size_of::<SdtHeader>() as u32) as *const u32;
    let entry_count = ((*rsdt).length as usize - size_of::<SdtHeader>()) / 4;

    let mut info = AcpiInfo {
        local_apic_addr: 0,
        ioapic_addr: 0,
        cpus: [const { None }; MAX_CPUS],
        cpu_count: 0,
    };

    for i in 0..entry_count {
        let table_addr = *entries_ptr.add(i);
        let header = &*(table_addr as *const SdtHeader);
        if &header.signature != b"APIC" {
            continue;
        }

        let madt = &*(table_addr as *const MadtHeader);
        info.local_apic_addr = madt.local_apic_addr;

        let mut p = table_addr + size_of::<MadtHeader>() as u32;
        let end = table_addr + madt.header.length;

        while p < end {
            let entry_type = *(p as *const u8);
            let entry_len = *((p + 1) as *const u8);

            match entry_type {
                0 => {
                    // Processor Local APIC
                    let apic_id = *((p + 3) as *const u8);
                    let flags = *((p + 4) as *const u32);
                    if flags & 1 != 0 && info.cpu_count < MAX_CPUS {
                        // enabled
                        info.cpus[info.cpu_count] = Some(CpuInfo { apic_id, bsp: false, flags });
                        smp::cpu::register_cpu(info.cpu_count, apic_id);
                        info.cpu_count += 1;
                    }
                }
                1 => {
                    // I/O APIC
                    info.ioapic_addr = *((p + 4) as *const u32);
                }
                _ => {}
            }
            p += entry_len as u32;
        }
        return Ok(info);
    }
    Err(AcpiError::MadtNotFound)
}
