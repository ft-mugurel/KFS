use super::SECTOR_SIZE;
use crate::error::{KResult, KernelError};
use crate::pr_warn;
use crate::x86::{inb, inw, outb, outw};

const IDE_PORT_BASE: u16 = 0x1F0;
const IDE_PORT_DATA: u16 = IDE_PORT_BASE + 0;
const IDE_PORT_SECT_COUNT: u16 = IDE_PORT_BASE + 2;
const IDE_PORT_LBA_LO: u16 = IDE_PORT_BASE + 3;
const IDE_PORT_LBA_MID: u16 = IDE_PORT_BASE + 4;
const IDE_PORT_LBA_HI: u16 = IDE_PORT_BASE + 5;
const IDE_PORT_DRV_HEAD: u16 = IDE_PORT_BASE + 6;
const IDE_PORT_COMMAND: u16 = IDE_PORT_BASE + 7;
const IDE_PORT_STATUS: u16 = IDE_PORT_BASE + 7;

// Status Register Flags
const STATUS_ERR: u8 = 0x01; // Error
const STATUS_DRQ: u8 = 0x08; // Data Request Ready
const STATUS_DF: u8 = 0x20; // Drive Fault Error
const STATUS_BSY: u8 = 0x80; // Busy

// ATA Commands
const CMD_READ_PIO: u8 = 0x20;
const CMD_WRITE_PIO: u8 = 0x30;
const CMD_CACHE_FLUSH: u8 = 0xE7;

fn wait_busy() -> KResult<()> {
    for _ in 0..4 {
        inb(IDE_PORT_STATUS);
    }

    // Poll until BSY clears
    loop {
        let status = inb(IDE_PORT_STATUS);
        if (status & STATUS_BSY) == 0 {
            if (status & STATUS_ERR) != 0 || (status & STATUS_DF) != 0 {
                return Err(KernelError::EIO); // I/O Error
            }
            return Ok(());
        }
    }
}

fn wait_drq() -> KResult<()> {
    loop {
        let status = inb(IDE_PORT_STATUS);
        if (status & STATUS_ERR) != 0 || (status & STATUS_DF) != 0 {
            return Err(KernelError::EIO);
        }
        if (status & STATUS_DRQ) != 0 {
            return Ok(());
        }
    }
}

pub fn read_sectors(lba: u32, sector_count: u8, buffer: &mut [u8]) -> KResult<()> {
    if buffer.len() < (sector_count as usize * SECTOR_SIZE) {
		pr_warn!("Buffer size is smaller than the requested sector count. Buffer size: {}, Requested size: {}", buffer.len(), sector_count as usize * SECTOR_SIZE);
        return Err(KernelError::EINVAL);
    }

    wait_busy()?;

    outb(IDE_PORT_DRV_HEAD, 0xE0 | ((lba >> 24) & 0x0F) as u8);
    outb(IDE_PORT_SECT_COUNT, sector_count);
    outb(IDE_PORT_LBA_LO, lba as u8);
    outb(IDE_PORT_LBA_MID, (lba >> 8) as u8);
    outb(IDE_PORT_LBA_HI, (lba >> 16) as u8);

    outb(IDE_PORT_COMMAND, CMD_READ_PIO);

    let buffer_ptr = buffer.as_mut_ptr() as *mut u16;

    for i in 0..(sector_count as usize) {
        wait_busy()?;
        wait_drq()?;

        for j in 0..256 {
            let word = inw(IDE_PORT_DATA);
            unsafe {
                buffer_ptr.add((i * 256) + j).write(word);
            }
        }
    }

    Ok(())
}

/// Writes a specific number of sectors starting at LBA from the provided buffer.
pub fn write_sectors(lba: u32, sector_count: u8, buffer: &[u8]) -> KResult<()> {
    if buffer.len() < (sector_count as usize * SECTOR_SIZE) {
        return Err(KernelError::EINVAL);
    }

    wait_busy()?;

    outb(IDE_PORT_DRV_HEAD, 0xE0 | ((lba >> 24) & 0x0F) as u8);
    outb(IDE_PORT_SECT_COUNT, sector_count);
    outb(IDE_PORT_LBA_LO, lba as u8);
    outb(IDE_PORT_LBA_MID, (lba >> 8) as u8);
    outb(IDE_PORT_LBA_HI, (lba >> 16) as u8);

    // Issue Write Command
    outb(IDE_PORT_COMMAND, CMD_WRITE_PIO);

    let buffer_ptr = buffer.as_ptr() as *const u16;

    for i in 0..(sector_count as usize) {
        wait_busy()?;
        wait_drq()?;

        // Write 256 words (512 bytes) for this sector
        for j in 0..256 {
            unsafe {
                let word = buffer_ptr.add((i * 256) + j).read();
                outw(IDE_PORT_DATA, word);
            }
        }
    }

    // Force drive cache flush
    flush_cache()?;

    Ok(())
}

pub fn flush_cache() -> KResult<()> {
    wait_busy()?;
    outb(IDE_PORT_COMMAND, CMD_CACHE_FLUSH);
    wait_busy()?;
    Ok(())
}