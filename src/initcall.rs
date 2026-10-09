use crate::error::KResultExt;
use crate::{paging, pr_debug, pr_err, pr_info, pr_warn};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum InitcallLevel {
    Early = 0,
    Core = 1,
    PostCore = 2,
    Arch = 3,
    Subsys = 4,
    Fs = 5,
    Device = 6,
    Late = 7,
}

impl InitcallLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            InitcallLevel::Early => "early",
            InitcallLevel::Core => "core",
            InitcallLevel::PostCore => "postcore",
            InitcallLevel::Arch => "arch",
            InitcallLevel::Subsys => "subsys",
            InitcallLevel::Fs => "fs",
            InitcallLevel::Device => "device",
            InitcallLevel::Late => "late",
        }
    }

    pub const fn from_u8(val: u8) -> Option<Self> {
        match val {
            0 => Some(InitcallLevel::Early),
            1 => Some(InitcallLevel::Core),
            2 => Some(InitcallLevel::PostCore),
            3 => Some(InitcallLevel::Arch),
            4 => Some(InitcallLevel::Subsys),
            5 => Some(InitcallLevel::Fs),
            6 => Some(InitcallLevel::Device),
            7 => Some(InitcallLevel::Late),
            _ => None,
        }
    }
}

pub trait IntoInitcallResult {
    fn into_initcall_result(self) -> i32;
}

impl IntoInitcallResult for () {
    fn into_initcall_result(self) -> i32 {
        0
    }
}

impl IntoInitcallResult for i32 {
    fn into_initcall_result(self) -> i32 {
        self
    }
}

impl<E: core::fmt::Debug> IntoInitcallResult for Result<(), E> {
    fn into_initcall_result(self) -> i32 {
        match self {
            Ok(()) => 0,
            Err(_) => -1,
        }
    }
}

pub type InitcallFn = fn() -> i32;

#[repr(C, align(4))]
#[derive(Clone, Copy)]
pub struct InitcallEntry {
    pub name: &'static str,
    pub func: InitcallFn,
    pub level: u8,
    pub _pad: [u8; 3],
}

#[macro_export]
macro_rules! define_initcall {
    ($level:expr, $sec:literal, $fn:path) => {
        const _: () = {
            #[unsafe(link_section = ".init.text")]
            #[allow(unused_unsafe)]
            fn __initcall_wrapper() -> i32 {
                unsafe { $crate::initcall::IntoInitcallResult::into_initcall_result($fn()) }
            }

            #[used]
            #[unsafe(link_section = $sec)]
            static __ENTRY: $crate::initcall::InitcallEntry = $crate::initcall::InitcallEntry {
                name: stringify!($fn),
                func: __initcall_wrapper,
                level: $level,
                _pad: [0; 3],
            };
        };
    };
}

#[macro_export]
macro_rules! pure_initcall {
    ($fn:path) => {
        $crate::define_initcall!(0, ".initcall0.init", $fn);
    };
}

#[macro_export]
macro_rules! early_initcall {
    ($fn:path) => {
        $crate::define_initcall!(0, ".initcall0.init", $fn);
    };
}

#[macro_export]
macro_rules! core_initcall {
    ($fn:path) => {
        $crate::define_initcall!(1, ".initcall1.init", $fn);
    };
}

#[macro_export]
macro_rules! postcore_initcall {
    ($fn:path) => {
        $crate::define_initcall!(2, ".initcall2.init", $fn);
    };
}

#[macro_export]
macro_rules! arch_initcall {
    ($fn:path) => {
        $crate::define_initcall!(3, ".initcall3.init", $fn);
    };
}

#[macro_export]
macro_rules! subsys_initcall {
    ($fn:path) => {
        $crate::define_initcall!(4, ".initcall4.init", $fn);
    };
}

#[macro_export]
macro_rules! fs_initcall {
    ($fn:path) => {
        $crate::define_initcall!(5, ".initcall5.init", $fn);
    };
}

#[macro_export]
macro_rules! device_initcall {
    ($fn:path) => {
        $crate::define_initcall!(6, ".initcall6.init", $fn);
    };
}

#[macro_export]
macro_rules! module_init {
    ($fn:path) => {
        $crate::define_initcall!(6, ".initcall6.init", $fn);
    };
}

#[macro_export]
macro_rules! rootfs_initcall {
    ($fn:path) => {
        $crate::define_initcall!(6, ".initcall6.init", $fn);
    };
}

#[macro_export]
macro_rules! late_initcall {
    ($fn:path) => {
        $crate::define_initcall!(7, ".initcall7.init", $fn);
    };
}

unsafe extern "C" {
    static __init_start: u8;
    static __init_end: u8;

    static __initcall_start: u8;
    static __initcall_end: u8;

    static __initcall0_start: u8;
    static __initcall0_end: u8;
    static __initcall1_start: u8;
    static __initcall1_end: u8;
    static __initcall2_start: u8;
    static __initcall2_end: u8;
    static __initcall3_start: u8;
    static __initcall3_end: u8;
    static __initcall4_start: u8;
    static __initcall4_end: u8;
    static __initcall5_start: u8;
    static __initcall5_end: u8;
    static __initcall6_start: u8;
    static __initcall6_end: u8;
    static __initcall7_start: u8;
    static __initcall7_end: u8;
}

unsafe fn slice_from_bounds(
    start: *const InitcallEntry,
    end: *const InitcallEntry,
) -> &'static [InitcallEntry] {
    let start_addr = start as usize;
    let end_addr = end as usize;
    if end_addr <= start_addr {
        return &[];
    }
    let entry_size = core::mem::size_of::<InitcallEntry>();
    let count = (end_addr - start_addr) / entry_size;
    core::slice::from_raw_parts(start, count)
}

pub fn all_initcalls() -> &'static [InitcallEntry] {
    unsafe {
        slice_from_bounds(
            core::ptr::addr_of!(__initcall_start) as *const InitcallEntry,
            core::ptr::addr_of!(__initcall_end) as *const InitcallEntry,
        )
    }
}

pub fn level_initcalls(level: u8) -> &'static [InitcallEntry] {
    unsafe {
        match level {
            0 => slice_from_bounds(
                core::ptr::addr_of!(__initcall0_start) as *const InitcallEntry,
                core::ptr::addr_of!(__initcall0_end) as *const InitcallEntry,
            ),
            1 => slice_from_bounds(
                core::ptr::addr_of!(__initcall1_start) as *const InitcallEntry,
                core::ptr::addr_of!(__initcall1_end) as *const InitcallEntry,
            ),
            2 => slice_from_bounds(
                core::ptr::addr_of!(__initcall2_start) as *const InitcallEntry,
                core::ptr::addr_of!(__initcall2_end) as *const InitcallEntry,
            ),
            3 => slice_from_bounds(
                core::ptr::addr_of!(__initcall3_start) as *const InitcallEntry,
                core::ptr::addr_of!(__initcall3_end) as *const InitcallEntry,
            ),
            4 => slice_from_bounds(
                core::ptr::addr_of!(__initcall4_start) as *const InitcallEntry,
                core::ptr::addr_of!(__initcall4_end) as *const InitcallEntry,
            ),
            5 => slice_from_bounds(
                core::ptr::addr_of!(__initcall5_start) as *const InitcallEntry,
                core::ptr::addr_of!(__initcall5_end) as *const InitcallEntry,
            ),
            6 => slice_from_bounds(
                core::ptr::addr_of!(__initcall6_start) as *const InitcallEntry,
                core::ptr::addr_of!(__initcall6_end) as *const InitcallEntry,
            ),
            7 => slice_from_bounds(
                core::ptr::addr_of!(__initcall7_start) as *const InitcallEntry,
                core::ptr::addr_of!(__initcall7_end) as *const InitcallEntry,
            ),
            _ => &[],
        }
    }
}

#[derive(Clone, Copy)]
pub struct InitcallRecord {
    pub name: &'static str,
    pub level: u8,
    pub result: i32,
    pub func_addr: u32,
}

const MAX_HISTORY: usize = 64;
static mut HISTORY: [InitcallRecord; MAX_HISTORY] =
    [InitcallRecord { name: "", level: 0, result: 0, func_addr: 0 }; MAX_HISTORY];
static mut HISTORY_COUNT: usize = 0;
static mut FREED_INIT_MEMORY: Option<(u32, u32, usize)> = None;

pub fn boot_history() -> &'static [InitcallRecord] {
    unsafe {
        let count = HISTORY_COUNT.min(MAX_HISTORY);
        &HISTORY[..count]
    }
}

pub fn freed_memory_info() -> Option<(u32, u32, usize)> {
    unsafe { FREED_INIT_MEMORY }
}

#[unsafe(link_section = ".init.text")]
pub fn do_initcalls() {
    pr_info!("initcall: starting kernel initialization calls\n");

    for level in 0..=7 {
        let entries = level_initcalls(level);
        if entries.is_empty() {
            continue;
        }

        let level_name = InitcallLevel::from_u8(level).map_or("unknown", |l| l.as_str());

        pr_debug!(
            "initcall: level {} ({}) [{} call(s)]\n",
            level,
            level_name,
            entries.len()
        );

        for entry in entries {
            let func_ptr = entry.func as usize as u32;
            pr_info!("[initcall:{}] {}...\n", level_name, entry.name);
            let ret = (entry.func)();

            unsafe {
                if HISTORY_COUNT < MAX_HISTORY {
                    HISTORY[HISTORY_COUNT] = InitcallRecord {
                        name: entry.name,
                        level: entry.level,
                        result: ret,
                        func_addr: func_ptr,
                    };
                    HISTORY_COUNT += 1;
                }
            }

            if ret != 0 {
                pr_err!(
                    "[initcall:{}] {} failed with error code {}\n",
                    level_name,
                    entry.name,
                    ret
                );
            } else {
                pr_debug!("[initcall:{}] {} ok\n", level_name, entry.name);
            }
        }
    }

    pr_info!("initcall: all initialization calls completed\n");
}

pub unsafe fn free_init_memory() {
    let start_addr = core::ptr::addr_of!(__init_start) as u32;
    let end_addr = core::ptr::addr_of!(__init_end) as u32;

    if end_addr <= start_addr {
        pr_warn!("initcall: no init memory section to free\n");
        return;
    }

    let page_size = 4096u32;
    let total_bytes = end_addr - start_addr;
    let total_pages = total_bytes / page_size;

    pr_info!(
        "Freeing unused kernel init memory: {:#010X} - {:#010X} ({} KiB, {} pages)\n",
        start_addr,
        end_addr,
        total_bytes / 1024,
        total_pages
    );

    let mut current_addr = start_addr;
    let mut freed_pages = 0usize;

    while current_addr < end_addr {
        if let Some(phys_addr) = paging::virt_to_phys(current_addr) {
            paging::free_physical_page(phys_addr)
                .consume_err("Failed to free physical page for init memory");

            paging::unmap_page(current_addr).consume_err("Failed to unmap page for init memory");

            // We only unmap the low virtual identity address (current_addr),
            // NOT the higher-half direct physical mapping alias (current_addr | 0xC000_0000).
            // phys_to_virt(phys) relies on [0xC000_0000, 0xC040_0000) remaining mapped
            // to physical [0, 4MB) when these freed physical frames are later reallocated
            // (e.g. for page directories, page tables, or process stacks).

            freed_pages += 1;
        } else {
            pr_warn!(
                "Failed to convert init virtual address to physical: {:#010X}\n",
                current_addr
            );
        }
        current_addr += page_size;
    }

    FREED_INIT_MEMORY = Some((start_addr, end_addr, freed_pages));
    pr_info!(
        "Init memory reclaimed: {} pages successfully freed\n",
        freed_pages
    );
}
