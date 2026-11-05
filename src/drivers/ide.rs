use super::SECTOR_SIZE;
use super::block_device::DiskKind;
use crate::error::{KResult, KernelError};
use crate::locks::{Spinlock, SpinlockGuard};
use crate::pr_warn;
use crate::x86::{inb, inw, outb, outw};

struct IdeChannels;
static IDE_LOCK: Spinlock<IdeChannels> = Spinlock::new(IdeChannels);

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

unsafe fn wait_busy(io_base: u16) -> KResult<()> {
    // The four reads give the drive ~400ns to settle after a command.
    for _ in 0..4 {
        unsafe { inb(io_base + 7) };
    }

    // Poll until BSY clears with timeout
    for _ in 0..MAX_POLL_RETRIES {
        let status = unsafe { inb(io_base + 7) };
        if (status & STATUS_BSY) == 0 {
            if (status & STATUS_ERR) != 0 || (status & STATUS_DF) != 0 {
                return Err(KernelError::IO); // I/O Error
            }
            return Ok(());
        }
    }
    pr_warn!("IDE wait_busy timed out on port {:#x}", io_base);
    Err(KernelError::TimedOut)
}

unsafe fn wait_drq(io_base: u16) -> KResult<()> {
    for _ in 0..MAX_POLL_RETRIES {
        let status = unsafe { inb(io_base + 7) };
        if (status & STATUS_ERR) != 0 || (status & STATUS_DF) != 0 {
            return Err(KernelError::IO);
        }
        if (status & STATUS_DRQ) != 0 {
            return Ok(());
        }
    }
    pr_warn!("IDE wait_drq timed out on port {:#x}", io_base);
    Err(KernelError::TimedOut)
}

pub unsafe fn identify(io_base: u16, control_base: u16, drive: u8) -> Option<(u32, DiskKind)> {
    let mut identify_data = [0u16; 256];
    unsafe {
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

        for word in &mut identify_data {
            *word = inw(io_base);
        }
    }

    let sector_count = (identify_data[61] as u32) << 16 | identify_data[60] as u32;
    if sector_count == 0 {
        return None;
    }

    // Word 76 is the SATA capabilities word; parallel ATA drives leave it 0 or 0xFFFF.
    let kind = match identify_data[76] {
        0 | 0xFFFF => DiskKind::Ide,
        _ => DiskKind::Scsi,
    };
    Some((sector_count, kind))
}

pub fn read_sectors(
    device: &super::BlockDevice,
    lba: u32,
    sector_count: u8,
    buffer: &mut [u8],
) -> KResult<()> {
    if buffer.len() < (sector_count as usize * SECTOR_SIZE) {
        pr_warn!(
            "Buffer size is smaller than the requested sector count. Buffer size: {}, Requested size: {}",
            buffer.len(),
            sector_count as usize * SECTOR_SIZE
        );
        return Err(KernelError::Inval);
    }

    let _guard = IDE_LOCK.lock();
    let io_base = device.io_base;

    unsafe {
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
    }

    for i in 0..(sector_count as usize) {
        unsafe {
            wait_busy(io_base)?;
            wait_drq(io_base)?;

            for chunk in buffer[(i * SECTOR_SIZE)..((i + 1) * SECTOR_SIZE)]
                .as_chunks_mut::<2>()
                .0
            {
                chunk.copy_from_slice(&inw(io_base).to_le_bytes());
            }
        }
    }

    Ok(())
}

pub fn write_sectors(
    device: &super::BlockDevice,
    lba: u32,
    sector_count: u8,
    buffer: &[u8],
) -> KResult<()> {
    if buffer.len() < (sector_count as usize * SECTOR_SIZE) {
        return Err(KernelError::Inval);
    }

    let guard = IDE_LOCK.lock();
    let io_base = device.io_base;

    unsafe {
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
    }

    for i in 0..(sector_count as usize) {
        unsafe {
            wait_busy(io_base)?;
            wait_drq(io_base)?;

            for chunk in buffer[(i * SECTOR_SIZE)..((i + 1) * SECTOR_SIZE)]
                .as_chunks::<2>()
                .0
            {
                outw(io_base, u16::from_le_bytes([chunk[0], chunk[1]]));
            }
        }
    }

    // Force drive cache flush
    flush_cache_locked(guard, device)?;

    Ok(())
}

fn flush_cache_locked(
    _held: SpinlockGuard<'_, IdeChannels>,
    device: &super::BlockDevice,
) -> KResult<()> {
    unsafe {
        wait_busy(device.io_base)?;
        outb(device.io_base + 6, device.drive);
        outb(device.io_base + 7, CMD_CACHE_FLUSH);
        wait_busy(device.io_base)?;
    }
    Ok(())
}
