mod ide;

pub const SECTOR_SIZE: usize = 512;

pub use ide::{read_sectors, write_sectors};