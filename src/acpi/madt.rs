use crate::paging::read_phys;
use crate::{pr_debug, pr_err, pr_info, pr_warn};

use crate::smp::{self, MAX_CPUS};

#[derive(Clone, Copy)]
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

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct MadtLocalApic {
    kind: u8,
    len: u8,
    processor_id: u8,
    apic_id: u8,
    flags: u32,
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

#[unsafe(link_section = ".init.text")]
pub fn init() -> Option<AcpiInfo> {
    let Ok(rsdp_addr) = (unsafe { find_rsdp() }) else {
        pr_err!("Failed to find RSDP");
        return None;
    };
    let Ok(info) = (unsafe { parse(rsdp_addr) }) else {
        pr_err!("Failed to parse ACPI tables");
        return None;
    };
    pr_debug!(
        "Found {} CPU(s), LAPIC @ {:#x}, IOAPIC @ {:#x}",
        info.cpu_count,
        info.local_apic_addr,
        info.ioapic_addr
    );
    info.cpus.iter().enumerate().for_each(|(i, cpu)| {
        if let Some(cpu_info) = cpu {
            pr_debug!(
                "CPU {}: APIC ID {}, Flags {:#x}",
                i,
                cpu_info.apic_id,
                cpu_info.flags
            );
        }
    });
    Some(info)
}

#[unsafe(link_section = ".init.text")]
unsafe fn find_rsdp() -> Result<u32, AcpiError> {
    let ebda_seg: u16 = unsafe { read_phys(0x40E) };
    let ebda_addr = (ebda_seg as u32) << 4;
    if let Ok(addr) = unsafe { scan_for_signature(ebda_addr, ebda_addr + 1024) } {
        return Ok(addr);
    } else {
        pr_info!("RSDP not found in EBDA, scanning 0xE0000-0x100000");
    }
    unsafe { scan_for_signature(0xE0000, 0x100000) }
}

#[unsafe(link_section = ".init.text")]
unsafe fn scan_for_signature(start: u32, end: u32) -> Result<u32, AcpiError> {
    let mut addr = start;
    while addr < end {
        let sig = unsafe { core::slice::from_raw_parts(addr as *const u8, 8) };
        if sig == b"RSD PTR " {
            if unsafe { checksum_ok(addr, size_of::<RsdpV1>()) } {
                return Ok(addr);
            } else {
                pr_warn!("Invalid checksum for RSDP at {:#x}", addr);
            }
        }
        addr += 16;
    }
    Err(AcpiError::RsdpNotFound)
}

#[unsafe(link_section = ".init.text")]
unsafe fn checksum_ok(addr: u32, len: usize) -> bool {
    let bytes = unsafe { core::slice::from_raw_parts(addr as *const u8, len) };
    let sum: u16 = bytes.iter().take(len).map(|f| (*f) as u16).sum();
    sum & 0xFF == 0
}

#[unsafe(link_section = ".init.text")]
unsafe fn parse(rspd_addr: u32) -> Result<AcpiInfo, AcpiError> {
    let rsdp: RsdpV1 = unsafe { read_phys(rspd_addr) };
    let madt = unsafe { find_table(rsdp.rsdt_addr, b"APIC") }?;
    Ok(unsafe { parse_madt(madt) })
}

#[inline(always)]
unsafe fn find_table(addr: u32, signature: &[u8; 4]) -> Result<u32, AcpiError> {
    let rsdt: SdtHeader = unsafe { read_phys(addr) };
    let header_len = size_of::<SdtHeader>() as u32;

    if rsdt.signature != *b"RSDT"
        || rsdt.length < header_len
        || unsafe { !checksum_ok(addr, rsdt.length as usize) }
    {
        return Err(AcpiError::InvalidChecksum);
    }

    let entry_count = (rsdt.length - header_len) / 4;
    let f_entry_addr = |ndx: u32| addr + header_len + ndx * 4;

    for i in 0..entry_count {
        let table_addr: u32 = unsafe { read_phys(f_entry_addr(i)) };
        let header: SdtHeader = unsafe { read_phys(table_addr) };
        if &header.signature == signature
            && header.length as usize >= size_of::<SdtHeader>()
            && unsafe { checksum_ok(table_addr, header.length as usize) }
        {
            return Ok(table_addr);
        }
    }
    Err(AcpiError::MadtNotFound)
}

#[inline(always)]
unsafe fn parse_madt(madt_addr: u32) -> AcpiInfo {
    let madt: MadtHeader = unsafe { read_phys(madt_addr) };
    let end = madt_addr + madt.header.length;
    let mut p: u32 = madt_addr + size_of::<MadtHeader>() as u32;
    let bsp_apic_id = (core::arch::x86::__cpuid(1).ebx >> 24) as u8;
    let mut next_ap_slot = 1;

    let mut info = AcpiInfo {
        local_apic_addr: madt.local_apic_addr,
        ioapic_addr: 0,
        cpus: [const { None }; MAX_CPUS],
        cpu_count: 0,
    };

    while p + 2 <= end {
        let (etype, len): (u8, u8) = unsafe { (read_phys(p), read_phys(p + 1)) };
        if len < 2 || p + len as u32 > end {
            break;
        }
        match etype {
            0 if len >= 8 => {
                let cpu: MadtLocalApic = unsafe { read_phys(p) };
                let bsp = cpu.apic_id == bsp_apic_id;
                let slot = if bsp { 0 } else { next_ap_slot };
                if (cpu.flags & 1 != 0) && slot < MAX_CPUS {
                    info.cpus[slot] = Some(CpuInfo { apic_id: cpu.apic_id, bsp, flags: cpu.flags });
                    smp::cpu::register_cpu(slot, cpu.apic_id);
                    info.cpu_count += 1;
                    if !bsp {
                        next_ap_slot += 1;
                    }
                }
            }
            1 if len >= 12 => info.ioapic_addr = unsafe { read_phys(p + 4) },
            _ => {}
        }
        p += len as u32;
    }
    info
}
