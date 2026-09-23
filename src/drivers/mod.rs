mod block_device;
mod ide;
mod pci;

use crate::error::KResult;

pub const SECTOR_SIZE: usize = 512;

pub use block_device::{
    device_at, read_sectors, write_sectors, BlockDevice, BlockDeviceId, MAX_BLOCK_DEVICES,
};

pub fn first_ext2_partition() -> Option<BlockDeviceId> {
    let mut superblock = [0u8; 1024];
    for device_id in 0..block_device::MAX_BLOCK_DEVICES {
        let Some(device) = block_device::device_at(device_id) else {
            continue;
        };
        if !device.partition {
            continue;
        }
        if read_sectors(device_id, 2, 2, &mut superblock).is_ok()
            && superblock[56] == 0x53
            && superblock[57] == 0xEF
        {
            return Some(device_id);
        }
    }
    None
}

#[unsafe(link_section = ".init.text")]
pub fn init() -> KResult<()> {
    pci::scan_and_register_block_devices()?;
    for device_id in 0..block_device::MAX_BLOCK_DEVICES {
        let Some(device) = block_device::device_at(device_id) else {
            continue;
        };
        if device.partition {
            continue;
        }
        let device_count = block_device::discover_mbr_partitions(device_id)?;
        if device_count == 0 {
            crate::pr_warn!(
                "No MBR partitions discovered on block device {}\n",
                device_id
            );
        }
    }
    Ok(())
}

crate::arch_initcall!(init);

