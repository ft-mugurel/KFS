use crate::error::KResult;
use crate::pr_info;
use crate::x86::{inl, outl};

use super::{block_device, ide, BlockDevice, SECTOR_SIZE};

const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
const PCI_CONFIG_DATA: u16 = 0xCFC;

const PCI_CLASS_STORAGE: u8 = 0x01;
const PCI_SUBCLASS_IDE: u8 = 0x01;

fn pci_config_address(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    (1 << 31)
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((function as u32) << 8)
        | ((offset as u32) & 0xFC)
}

fn read_config_dword(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    outl(
        PCI_CONFIG_ADDRESS,
        pci_config_address(bus, device, function, offset),
    );
    inl(PCI_CONFIG_DATA)
}

fn read_vendor_id(bus: u8, device: u8, function: u8) -> u16 {
    (read_config_dword(bus, device, function, 0x00) & 0xFFFF) as u16
}

fn read_class_info(bus: u8, device: u8, function: u8) -> (u8, u8, u8) {
    let value = read_config_dword(bus, device, function, 0x08);
    let class_code = (value >> 24) as u8;
    let subclass = (value >> 16) as u8;
    let prog_if = (value >> 8) as u8;
    (class_code, subclass, prog_if)
}

fn read_bar(bus: u8, device: u8, function: u8, index: u8) -> u16 {
    (read_config_dword(bus, device, function, 0x10 + index * 4) & 0xFFFC) as u16
}

fn register_ide_drive(io_base: u16, control_base: u16, drive: u8, name_index: u8) -> KResult<bool> {
    let Some(sector_count) = ide::identify(io_base, control_base, drive) else {
        return Ok(false);
    };

    let mut name = [0u8; 32];
    name[..3].copy_from_slice(b"ide");
    name[3] = b'0' + name_index;
    block_device::register_block_device(BlockDevice {
        name,
        sector_size: SECTOR_SIZE,
        parent: 0,
        start_lba: 0,
        sector_count,
        partition: false,
        io_base,
        drive,
        read: ide::read_sectors,
        write: ide::write_sectors,
    })?;
    Ok(true)
}

pub fn scan_and_register_block_devices() -> KResult<()> {
    let mut ide_count = 0u8;

    for bus in 0u8..=u8::MAX {
        for device in 0u8..32 {
            for function in 0u8..8 {
                if read_vendor_id(bus, device, function) == 0xFFFF {
                    continue;
                }

                let (class_code, subclass, prog_if) = read_class_info(bus, device, function);
                if class_code != PCI_CLASS_STORAGE || subclass != PCI_SUBCLASS_IDE {
                    continue;
                }

                let primary_io = if prog_if & 1 != 0 {
                    read_bar(bus, device, function, 0)
                } else {
                    0x1F0
                };
                let primary_control = if prog_if & 1 != 0 {
                    read_bar(bus, device, function, 1)
                } else {
                    0x3F4
                };
                let secondary_io = if prog_if & 4 != 0 {
                    read_bar(bus, device, function, 2)
                } else {
                    0x170
                };
                let secondary_control = if prog_if & 4 != 0 {
                    read_bar(bus, device, function, 3)
                } else {
                    0x374
                };

                for (io_base, control_base) in [
                    (primary_io, primary_control),
                    (secondary_io, secondary_control),
                ] {
                    for drive in [0xA0, 0xB0] {
                        if register_ide_drive(io_base, control_base, drive, ide_count)? {
                            ide_count += 1;
                        }
                    }
                }

                pr_info!(
                    "PCI: IDE controller {:02x}:{:02x}.{:x} discovered (prog_if={:02x})\n",
                    bus,
                    device,
                    function,
                    prog_if
                );
            }
        }
    }

    if ide_count == 0 {
        for (io_base, control_base) in [(0x1F0, 0x3F4), (0x170, 0x374)] {
            for drive in [0xA0, 0xB0] {
                if register_ide_drive(io_base, control_base, drive, ide_count)? {
                    ide_count += 1;
                }
            }
        }
        pr_info!("PCI: no IDE controller discovered, falling back to legacy IDE ports\n");
    }

    Ok(())
}
