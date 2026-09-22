use super::SECTOR_SIZE;
use crate::error::{KResult, KernelError};
use crate::pr_warn;
use crate::x86::{inb, inw, outb, outw};

const IDE_CONTROL_ALT_STATUS: u16 = 2;

// Status Register Flags
const STATUS_ERR: u8 = 0x01; // Error
const STATUS_DRQ: u8 = 0x08; // Data Request Ready
const STATUS_DF: u8 = 0x20; // Drive Fault Error
const STATUS_BSY: u8 = 0x80; // Busy

// ATA Commands
const CMD_READ_PIO: u8 = 0x20;
const CMD_WRITE_PIO: u8 = 0x30;
const CMD_CACHE_FLUSH: u8 = 0xE7;
const CMD_IDENTIFY: u8 = 0xEC;

const MAX_POLL_RETRIES: u32 = 1_000_000;

fn wait_busy(io_base: u16) -> KResult<()> {
    for _ in 0..4 {
        inb(io_base + 7);
    }

    // Poll until BSY clears with timeout
    for _ in 0..MAX_POLL_RETRIES {
        let status = inb(io_base + 7);
        if (status & STATUS_BSY) == 0 {
            if (status & STATUS_ERR) != 0 || (status & STATUS_DF) != 0 {
                return Err(KernelError::EIO); // I/O Error
            }
            return Ok(());
        }
    }
    pr_warn!("IDE wait_busy timed out on port {:#x}", io_base);
    Err(KernelError::ETIMEDOUT)
}

pub fn identify(io_base: u16, control_base: u16, drive: u8) -> Option<u32> {
    outb(io_base + 6, drive);
    for _ in 0..4 {
        inb(control_base + IDE_CONTROL_ALT_STATUS);
    }
    let status = inb(io_base + 7);
    if status == 0 || status == 0xFF {
        return None;
    }

    outb(io_base + 2, 0);
    outb(io_base + 3, 0);
    outb(io_base + 4, 0);
    outb(io_base + 5, 0);
    outb(io_base + 7, CMD_IDENTIFY);

    let status = inb(io_base + 7);
    if status == 0 || status == 0xFF {
        return None;
    }
    wait_busy(io_base).ok()?;
    wait_drq(io_base).ok()?;

    let mut identify_data = [0u16; 256];
    for word in &mut identify_data {
        *word = inw(io_base);
    }

    let sector_count = (identify_data[61] as u32) << 16 | identify_data[60] as u32;
    if sector_count == 0 {
        None
    } else {
        Some(sector_count)
    }
}

fn wait_drq(io_base: u16) -> KResult<()> {
    for _ in 0..MAX_POLL_RETRIES {
        let status = inb(io_base + 7);
        if (status & STATUS_ERR) != 0 || (status & STATUS_DF) != 0 {
            return Err(KernelError::EIO);
        }
        if (status & STATUS_DRQ) != 0 {
            return Ok(());
        }
    }
    pr_warn!("IDE wait_drq timed out on port {:#x}", io_base);
    Err(KernelError::ETIMEDOUT)
}

pub fn read_sectors(
    device: &super::BlockDevice,
    lba: u32,
    sector_count: u8,
    buffer: &mut [u8],
) -> KResult<()> {
    if buffer.len() < (sector_count as usize * SECTOR_SIZE) {
        pr_warn!("Buffer size is smaller than the requested sector count. Buffer size: {}, Requested size: {}", buffer.len(), sector_count as usize * SECTOR_SIZE);
        return Err(KernelError::EINVAL);
    }

    let io_base = device.io_base;
    wait_busy(io_base)?;

    outb(
        io_base + 6,
        device.drive | 0x40 | ((lba >> 24) & 0x0F) as u8,
    );
    outb(io_base + 2, sector_count);
    outb(io_base + 3, lba as u8);
    outb(io_base + 4, (lba >> 8) as u8);
    outb(io_base + 5, (lba >> 16) as u8);

    outb(io_base + 7, CMD_READ_PIO);

    let buffer_ptr = buffer.as_mut_ptr() as *mut u16;

    for i in 0..(sector_count as usize) {
        wait_busy(io_base)?;
        wait_drq(io_base)?;

        for j in 0..256 {
            let word = inw(io_base);
            unsafe {
                buffer_ptr.add((i * 256) + j).write(word);
            }
        }
    }

    Ok(())
}

/// Writes a specific number of sectors starting at LBA from the provided buffer.
pub fn write_sectors(
    device: &super::BlockDevice,
    lba: u32,
    sector_count: u8,
    buffer: &[u8],
) -> KResult<()> {
    if buffer.len() < (sector_count as usize * SECTOR_SIZE) {
        return Err(KernelError::EINVAL);
    }

    let io_base = device.io_base;
    wait_busy(io_base)?;

    outb(
        io_base + 6,
        device.drive | 0x40 | ((lba >> 24) & 0x0F) as u8,
    );
    outb(io_base + 2, sector_count);
    outb(io_base + 3, lba as u8);
    outb(io_base + 4, (lba >> 8) as u8);
    outb(io_base + 5, (lba >> 16) as u8);

    // Issue Write Command
    outb(io_base + 7, CMD_WRITE_PIO);

    let buffer_ptr = buffer.as_ptr() as *const u16;

    for i in 0..(sector_count as usize) {
        wait_busy(io_base)?;
        wait_drq(io_base)?;

        // Write 256 words (512 bytes) for this sector
        for j in 0..256 {
            unsafe {
                let word = buffer_ptr.add((i * 256) + j).read();
                outw(io_base, word);
            }
        }
    }

    // Force drive cache flush
    flush_cache(device)?;

    Ok(())
}

pub fn flush_cache(device: &super::BlockDevice) -> KResult<()> {
    wait_busy(device.io_base)?;
    outb(device.io_base + 6, device.drive);
    outb(device.io_base + 7, CMD_CACHE_FLUSH);
    wait_busy(device.io_base)?;
    Ok(())
}
