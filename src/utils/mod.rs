mod str;

pub struct MemUtils {}

impl MemUtils {
    pub const fn region_contains((base, size): (u32, u32), x: u32) -> bool {
        x >= base && x - base < size
    }
}

pub use str::*;
