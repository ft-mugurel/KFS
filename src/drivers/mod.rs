mod block_device;
mod ide;
mod pci;

use crate::error::KResult;

pub const SECTOR_SIZE: usize = 512;

pub use block_device::{read_sectors, write_sectors, BlockDevice, BlockDeviceId};

pub fn init() -> KResult<()> {
    pci::scan_and_register_block_devices()
}
