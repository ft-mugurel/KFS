use crate::{
    error::{KResult, KernelError},
    locks::Spinlock,
    pr_info,
};

pub type BlockReadFn =
    fn(device: &BlockDevice, lba: u32, sector_count: u8, buffer: &mut [u8]) -> KResult<()>;
pub type BlockWriteFn =
    fn(device: &BlockDevice, lba: u32, sector_count: u8, buffer: &[u8]) -> KResult<()>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskKind {
    Ide,
    Scsi,
    Virtio,
}

impl DiskKind {
    const fn prefix(self) -> &'static [u8; 2] {
        match self {
            DiskKind::Ide => b"hd",
            DiskKind::Scsi => b"sd",
            DiskKind::Virtio => b"vd",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BlockDevice {
    pub name: [u8; 32],
    pub sector_size: usize,
    pub parent: BlockDeviceId,
    pub start_lba: u32,
    pub sector_count: u32,
    pub partition: bool,
    pub(super) kind: DiskKind,
    pub(super) io_base: u16,
    pub(super) drive: u8,
    pub(super) read: BlockReadFn,
    pub(super) write: BlockWriteFn,
}

pub type BlockDeviceId = usize;

pub const MAX_BLOCK_DEVICES: usize = 16;

static BLOCK_DEVICE_TABLE: Spinlock<[Option<BlockDevice>; MAX_BLOCK_DEVICES]> = {
    const EMPTY: Option<BlockDevice> = None;
    Spinlock::new([EMPTY; MAX_BLOCK_DEVICES])
};

/// Next free whole-disk name for `kind`: letters count up per kind (hda, hdb, vda).
pub(super) fn next_disk_name(kind: DiskKind) -> [u8; 32] {
    let table = BLOCK_DEVICE_TABLE.lock();
    let index = table
        .iter()
        .flatten()
        .filter(|device| !device.partition && device.kind == kind)
        .count();

    let mut name = [0u8; 32];
    name[..2].copy_from_slice(kind.prefix());
    name[2] = b'a' + index as u8;
    name
}

pub(super) fn register_block_device(device: BlockDevice) -> KResult<BlockDeviceId> {
    let mut table = BLOCK_DEVICE_TABLE.lock();
    for (index, entry) in table.iter_mut().enumerate() {
        if entry.is_none() {
            *entry = Some(device);
            pr_info!(
                "Registered block device: {} (sector size: {}) at index {}",
                core::str::from_utf8(&device.name)
                    .map(|s| s.trim_end_matches('\0'))
                    .unwrap_or("Invalid UTF-8"),
                device.sector_size,
                index
            );
            return Ok(index);
        }
    }

    Err(KernelError::NFile)
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
    let device = device_at(device_id).ok_or(KernelError::NoDev)?;
    let required = (sector_count as usize) * device.sector_size;
    if buffer.len() < required {
        return Err(KernelError::Inval);
    }
    if (lba as u64) + (sector_count as u64) > device.sector_count as u64 {
        return Err(KernelError::Inval);
    }
    (device.read)(&device, lba, sector_count, buffer)
}

pub fn write_sectors(
    device_id: BlockDeviceId,
    lba: u32,
    sector_count: u8,
    buffer: &[u8],
) -> KResult<()> {
    let device = device_at(device_id).ok_or(KernelError::NoDev)?;
    let required = (sector_count as usize) * device.sector_size;
    if buffer.len() < required {
        return Err(KernelError::Inval);
    }
    if (lba as u64) + (sector_count as u64) > device.sector_count as u64 {
        return Err(KernelError::Inval);
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
        return Err(KernelError::Inval);
    }
    let parent = device_at(device.parent).ok_or(KernelError::NoDev)?;
    (parent.read)(
        &parent,
        device
            .start_lba
            .checked_add(lba)
            .ok_or(KernelError::Overflow)?,
        sector_count,
        buffer,
    )
}

fn partition_write(device: &BlockDevice, lba: u32, sector_count: u8, buffer: &[u8]) -> KResult<()> {
    let end = (lba as u64) + (sector_count as u64);
    if end > device.sector_count as u64 {
        return Err(KernelError::Inval);
    }
    let parent = device_at(device.parent).ok_or(KernelError::NoDev)?;
    (parent.write)(
        &parent,
        device
            .start_lba
            .checked_add(lba)
            .ok_or(KernelError::Overflow)?,
        sector_count,
        buffer,
    )
}

pub fn discover_mbr_partitions(device_id: BlockDeviceId) -> KResult<usize> {
    let device = device_at(device_id).ok_or(KernelError::NoDev)?;
    let mut sector = [0u8; 512];
    (device.read)(&device, 0, 1, &mut sector)?;

    if sector[510] != 0x55 || sector[511] != 0xAA {
        pr_info!("Block device {} has no valid MBR", device_id);
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
                "Skipping partition {} on device {}: outside disk capacity",
                index + 1,
                device_id
            );
            continue;
        }

        let mut name = device.name;
        let name_len = name
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(name.len() - 1);
        name[name_len] = b'1' + index as u8;
        let partition_id = register_block_device(BlockDevice {
            name,
            sector_size: device.sector_size,
            parent: device_id,
            start_lba,
            sector_count,
            partition: true,
            kind: device.kind,
            io_base: 0,
            drive: 0,
            read: partition_read,
            write: partition_write,
        })?;
        pr_info!(
            "Registered partition {}: type {:#x}, start {}, sectors {}",
            partition_id,
            partition_type,
            start_lba,
            sector_count
        );
        found += 1;
    }

    Ok(found)
}

pub fn read_from_device(device_id: u32, buffer: &mut [u8], offset: u32) -> KResult<usize> {
    let device = device_at(device_id as usize).ok_or(KernelError::NoDev)?;
    let sector_size = device.sector_size;

    if sector_size == 0 || sector_size > 512 {
        return Err(KernelError::Inval);
    }

    if buffer.is_empty() {
        return Ok(0);
    }

    let total_bytes = (device.sector_count as u64) * (sector_size as u64);
    if (offset as u64) >= total_bytes {
        return Ok(0);
    }

    let mut temp = [0u8; 512];
    let mut bytes_read = 0usize;
    let mut current_offset = offset as usize;
    let max_read = core::cmp::min(buffer.len(), (total_bytes - offset as u64) as usize);

    while bytes_read < max_read {
        let sector_index = current_offset / sector_size;
        let offset_in_sector = current_offset % sector_size;
        let bytes_available = core::cmp::min(sector_size - offset_in_sector, max_read - bytes_read);

        read_sectors(
            device_id as usize,
            sector_index as u32,
            1,
            &mut temp[..sector_size],
        )?;

        buffer[bytes_read..bytes_read + bytes_available]
            .copy_from_slice(&temp[offset_in_sector..offset_in_sector + bytes_available]);

        bytes_read += bytes_available;
        current_offset += bytes_available;
    }

    Ok(bytes_read)
}

pub fn write_to_device(device_id: u32, buffer: &[u8], offset: u32) -> KResult<usize> {
    let device = device_at(device_id as usize).ok_or(KernelError::NoDev)?;
    let sector_size = device.sector_size;

    if sector_size == 0 || sector_size > 512 {
        return Err(KernelError::Inval);
    }

    if buffer.is_empty() {
        return Ok(0);
    }

    let total_bytes = (device.sector_count as u64) * (sector_size as u64);
    if (offset as u64) >= total_bytes {
        return Err(KernelError::NoSpc);
    }

    let mut temp = [0u8; 512];
    let mut bytes_written = 0usize;
    let mut current_offset = offset as usize;
    let max_write = core::cmp::min(buffer.len(), (total_bytes - offset as u64) as usize);

    while bytes_written < max_write {
        let sector_index = current_offset / sector_size;
        let offset_in_sector = current_offset % sector_size;
        let bytes_available =
            core::cmp::min(sector_size - offset_in_sector, max_write - bytes_written);
        let sector_slice = &mut temp[..sector_size];

        if offset_in_sector != 0 || bytes_available != sector_size {
            read_sectors(device_id as usize, sector_index as u32, 1, sector_slice)?;
        }

        sector_slice[offset_in_sector..offset_in_sector + bytes_available]
            .copy_from_slice(&buffer[bytes_written..bytes_written + bytes_available]);

        write_sectors(device_id as usize, sector_index as u32, 1, sector_slice)?;

        bytes_written += bytes_available;
        current_offset += bytes_available;
    }

    Ok(bytes_written)
}
