#[repr(C)]
pub struct MultibootInfo {
    pub flags: u32,
    pub mem_lower: u32,
    pub mem_upper: u32,
    pub boot_device: u32,
    pub cmdline: u32,
    pub mods_count: u32,
    pub mods_addr: u32,
    pub syms: [u32; 4],
    pub mmap_length: u32,
    pub mmap_addr: u32,
    pub drives_length: u32,
    pub drives_addr: u32,
    pub boot_loader_name: u32,
    pub apm_table: u32,
    pub vbe_control_info: u32,
    pub vbe_mode_info: u32,
    pub vbe_mode: u16,
    pub vbe_interface_seg: u16,
    pub vbe_interface_off: u16,
    pub vbe_interface_len: u16,
    pub framebuffer_addr: u64,
    pub framebuffer_pitch: u32,
    pub framebuffer_width: u32,
    pub framebuffer_height: u32,
    pub framebuffer_bpp: u8,
    pub framebuffer_type: u8,
    pub color_info: [u8; 6],
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MultibootMmapEntry {
    pub size: u32,
    pub addr: u64,
    pub len: u64,
    pub entry_type: u32,
}

#[derive(Clone, Copy)]
pub struct PhysicalMemoryRegion {
    pub base_addr: u64,
    pub length: u64,
    pub region_type: u32,
}

pub const MULTIBOOT_BOOTLOADER_MAGIC: u32 = 0x2BADB002;
pub const MULTIBOOT_INFO_HAS_BASIC_MEMORY: u32 = 1 << 0;
pub const MULTIBOOT_INFO_HAS_MMAP: u32 = 1 << 6;
pub const MULTIBOOT_MEMORY_AVAILABLE: u32 = 1;

static mut BOOT_MULTIBOOT_INFO_ADDR: u32 = 0;

pub fn multiboot_info_from_addr(addr: u32) -> &'static MultibootInfo {
    unsafe { &*(addr as usize as *const MultibootInfo) }
}

pub fn set_boot_multiboot_info_addr(addr: u32) {
    unsafe {
        BOOT_MULTIBOOT_INFO_ADDR = addr;
    }
}

pub struct MemoryMapIter {
    current: usize,
    end: usize,
}

impl MemoryMapIter {
    pub fn new(info: &MultibootInfo) -> Option<Self> {
        if (info.flags & MULTIBOOT_INFO_HAS_MMAP) == 0 {
            return None;
        }

        let start = info.mmap_addr as usize;
        let end = start.saturating_add(info.mmap_length as usize);
        Some(Self { current: start, end })
    }
}

impl Iterator for MemoryMapIter {
    type Item = PhysicalMemoryRegion;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current >= self.end {
            return None;
        }

        let entry_ptr = self.current as *const MultibootMmapEntry;
        let entry = unsafe { core::ptr::read_unaligned(entry_ptr) };

        let full_size = entry.size as usize + core::mem::size_of::<u32>();
        if full_size == 0 {
            return None;
        }

        self.current = self.current.saturating_add(full_size);

        Some(PhysicalMemoryRegion {
            base_addr: entry.addr,
            length: entry.len,
            region_type: entry.entry_type,
        })
    }
}
