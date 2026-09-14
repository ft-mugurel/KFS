use crate::error::{KResult, KernelError};
use crate::locks::Spinlock;
use crate::pr_info;

pub type BlockReadFn =
    fn(device: &BlockDevice, lba: u32, sector_count: u8, buffer: &mut [u8]) -> KResult<()>;
pub type BlockWriteFn =
    fn(device: &BlockDevice, lba: u32, sector_count: u8, buffer: &[u8]) -> KResult<()>;

#[derive(Debug, Clone, Copy)]
pub struct BlockDevice {
    pub name: [u8; 32],
    pub sector_size: usize,
    pub parent: BlockDeviceId,
    pub start_lba: u32,
    pub sector_count: u32,
    pub partition: bool,
    pub io_base: u16,
    pub drive: u8,
    pub read: BlockReadFn,
    pub write: BlockWriteFn,
}

pub type BlockDeviceId = usize;

pub const MAX_BLOCK_DEVICES: usize = 16;

static BLOCK_DEVICE_TABLE: Spinlock<[Option<BlockDevice>; MAX_BLOCK_DEVICES]> = {
    const EMPTY: Option<BlockDevice> = None;
    Spinlock::new([EMPTY; MAX_BLOCK_DEVICES])
};

pub fn register_block_device(device: BlockDevice) -> KResult<BlockDeviceId> {
    let mut table = BLOCK_DEVICE_TABLE.lock();
    for (index, entry) in table.iter_mut().enumerate() {
        if entry.is_none() {
            *entry = Some(device);
            pr_info!(
                "Registered block device: {} (sector size: {}) at index {}\n",
                core::str::from_utf8(&device.name).unwrap_or("Invalid UTF-8"),
                device.sector_size,
                index
            );
            return Ok(index);
        }
    }

    Err(KernelError::ENFILE)
}

pub fn device_at(index: usize) -> Option<BlockDevice> {
    if index >= MAX_BLOCK_DEVICES {
        return None;
    }
    let table = BLOCK_DEVICE_TABLE.lock();
    table[index]
}

pub fn read_sectors(
    device_id: BlockDeviceId,
    lba: u32,
    sector_count: u8,
    buffer: &mut [u8],
) -> KResult<()> {
    let device = device_at(device_id).ok_or(KernelError::ENODEV)?;
    let required = (sector_count as usize) * device.sector_size;
    if buffer.len() < required {
        return Err(KernelError::EINVAL);
    }
    if (lba as u64) + (sector_count as u64) > device.sector_count as u64 {
        return Err(KernelError::EINVAL);
    }
    (device.read)(&device, lba, sector_count, buffer)
}

pub fn write_sectors(
    device_id: BlockDeviceId,
    lba: u32,
    sector_count: u8,
    buffer: &[u8],
) -> KResult<()> {
    let device = device_at(device_id).ok_or(KernelError::ENODEV)?;
    let required = (sector_count as usize) * device.sector_size;
    if buffer.len() < required {
        return Err(KernelError::EINVAL);
    }
    if (lba as u64) + (sector_count as u64) > device.sector_count as u64 {
        return Err(KernelError::EINVAL);
    }
    (device.write)(&device, lba, sector_count, buffer)
}

fn partition_read(
    device: &BlockDevice,
    lba: u32,
    sector_count: u8,
    buffer: &mut [u8],
) -> KResult<()> {
    let end = (lba as u64) + (sector_count as u64);
    if end > device.sector_count as u64 {
        return Err(KernelError::EINVAL);
    }
    let parent = device_at(device.parent).ok_or(KernelError::ENODEV)?;
    (parent.read)(
        &parent,
        device
            .start_lba
            .checked_add(lba)
            .ok_or(KernelError::EOVERFLOW)?,
        sector_count,
        buffer,
    )
}

fn partition_write(device: &BlockDevice, lba: u32, sector_count: u8, buffer: &[u8]) -> KResult<()> {
    let end = (lba as u64) + (sector_count as u64);
    if end > device.sector_count as u64 {
        return Err(KernelError::EINVAL);
    }
    let parent = device_at(device.parent).ok_or(KernelError::ENODEV)?;
    (parent.write)(
        &parent,
        device
            .start_lba
            .checked_add(lba)
            .ok_or(KernelError::EOVERFLOW)?,
        sector_count,
        buffer,
    )
}

pub fn discover_mbr_partitions(device_id: BlockDeviceId) -> KResult<usize> {
    let device = device_at(device_id).ok_or(KernelError::ENODEV)?;
    let mut sector = [0u8; 512];
    (device.read)(&device, 0, 1, &mut sector)?;

    if sector[510] != 0x55 || sector[511] != 0xAA {
        pr_info!("Block device {} has no valid MBR\n", device_id);
        return Ok(0);
    }

    let mut found = 0;
    for index in 0..4 {
        let offset = 446 + index * 16;
        let partition_type = sector[offset + 4];
        let start_lba = u32::from_le_bytes([
            sector[offset + 8],
            sector[offset + 9],
            sector[offset + 10],
            sector[offset + 11],
        ]);
        let sector_count = u32::from_le_bytes([
            sector[offset + 12],
            sector[offset + 13],
            sector[offset + 14],
            sector[offset + 15],
        ]);

        if partition_type == 0 || sector_count == 0 {
            continue;
        }
        if start_lba >= device.sector_count || sector_count > device.sector_count - start_lba {
            pr_info!(
                "Skipping partition {} on device {}: outside disk capacity\n",
                index + 1,
                device_id
            );
            continue;
        }

        let mut name = [0u8; 32];
        name[..2].copy_from_slice(b"hd");
        name[2] = b'a' + (device_id as u8);
        name[3] = b'1' + index as u8;
        let partition_id = register_block_device(BlockDevice {
            name,
            sector_size: device.sector_size,
            parent: device_id,
            start_lba,
            sector_count,
            partition: true,
            io_base: 0,
            drive: 0,
            read: partition_read,
            write: partition_write,
        })?;
        pr_info!(
            "Registered partition {}: type {:#x}, start {}, sectors {}\n",
            partition_id,
            partition_type,
            start_lba,
            sector_count
        );
        found += 1;
    }

    Ok(found)
}

/* pub unsafe fn read_from_device(device_id: u32, buffer: &mut [u8], offset: u32) -> KResult<usize> {
    let device = device_at(device_id as usize).ok_or(KernelError::ENODEV)?;
    let sector_size = device.sector_size;

    if sector_size == 0 || sector_size > 4096 {
        return Err(KernelError::EINVAL);
    }

    if buffer.is_empty() {
        return Ok(0);
    }

    let mut temp = [0u8; 4096];
    let mut bytes_read = 0usize;
    let mut current_offset = offset as usize;

    while bytes_read < buffer.len() {
        let sector_index = current_offset / sector_size;
        let offset_in_sector = current_offset % sector_size;
        let bytes_available =
            core::cmp::min(sector_size - offset_in_sector, buffer.len() - bytes_read);

        (device.read)(sector_index as u32, 1, &mut temp[..sector_size])?;

        buffer[bytes_read..bytes_read + bytes_available]
            .copy_from_slice(&temp[offset_in_sector..offset_in_sector + bytes_available]);

        bytes_read += bytes_available;
        current_offset += bytes_available;
    }

    Ok(bytes_read)
}

pub unsafe fn write_to_device(device_id: u32, buffer: &[u8], offset: u32) -> KResult<usize> {
    let device = device_at(device_id as usize).ok_or(KernelError::ENODEV)?;
    let sector_size = device.sector_size;

    if sector_size == 0 || sector_size > 4096 {
        return Err(KernelError::EINVAL);
    }

    if buffer.is_empty() {
        return Ok(0);
    }

    let mut temp = [0u8; 4096];
    let mut bytes_written = 0usize;
    let mut current_offset = offset as usize;

    while bytes_written < buffer.len() {
        let sector_index = current_offset / sector_size;
        let offset_in_sector = current_offset % sector_size;
        let bytes_available =
            core::cmp::min(sector_size - offset_in_sector, buffer.len() - bytes_written);
        let sector_slice = &mut temp[..sector_size];

        if offset_in_sector != 0 || bytes_available != sector_size {
            (device.read)(sector_index as u32, 1, sector_slice)?;
        }

        sector_slice[offset_in_sector..offset_in_sector + bytes_available]
            .copy_from_slice(&buffer[bytes_written..bytes_written + bytes_available]);

        (device.write)(sector_index as u32, 1, sector_slice)?;

        bytes_written += bytes_available;
        current_offset += bytes_available;
    }

    Ok(bytes_written)
}
 */
