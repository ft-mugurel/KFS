use core::fmt::Write;

use crate::{
    drivers, dump,
    error::{KResult, KernelError},
    fs::{self, FsBackend, VfsNode, VfsNodeType},
    modules::{self, ModuleState},
    smp,
};

use super::BufferWriter;

unsafe fn sysfs_create(
    _mount: *const fs::Mount,
    _parent_inode: u32,
    _name: &str,
    _node_type: VfsNodeType,
    _mode: u16,
    _uid: u32,
    _gid: u32,
) -> KResult<u32> {
    Err(KernelError::ROFS)
}

#[derive(Clone, Copy)]
enum Attr {
    MemStat,
    MemDebug,
    Initcalls,
    SysInfo,
    CpuOnline,
    CpuPossible,
    CpuPresent,
    ModuleState(usize),
    BlockSize(drivers::BlockDeviceId),
    BlockStart(drivers::BlockDeviceId),
}

fn decode(inode: u32) -> Option<Attr> {
    use super::*;
    Some(match inode {
        SYSFS_DEBUG_MEMSTAT_INODE => Attr::MemStat,
        SYSFS_DEBUG_MEMDEBUG_INODE => Attr::MemDebug,
        SYSFS_DEBUG_INITCALLS_INODE => Attr::Initcalls,
        SYSFS_DEBUG_SYSINFO_INODE => Attr::SysInfo,
        SYSFS_CPU_ONLINE_INODE => Attr::CpuOnline,
        SYSFS_CPU_POSSIBLE_INODE => Attr::CpuPossible,
        SYSFS_CPU_PRESENT_INODE => Attr::CpuPresent,
        inode if inode >= SYSFS_BLOCK_BASE => {
            let rel = inode - SYSFS_BLOCK_BASE;
            let device = (rel / SYSFS_BLOCK_STRIDE) as usize;
            match rel % SYSFS_BLOCK_STRIDE {
                SYSFS_BLOCK_SIZE => Attr::BlockSize(device),
                SYSFS_BLOCK_START => Attr::BlockStart(device),
                _ => return None,
            }
        }
        inode if inode >= SYSFS_MODULE_BASE => {
            let rel = inode - SYSFS_MODULE_BASE;
            let index = (rel / SYSFS_MODULE_STRIDE) as usize;
            if index >= modules::builtin_count() {
                return None;
            }
            match rel % SYSFS_MODULE_STRIDE {
                SYSFS_MODULE_STATE => Attr::ModuleState(index),
                _ => return None,
            }
        }
        _ => return None,
    })
}

fn parse_value(buffer: &[u8]) -> KResult<&str> {
    Ok(core::str::from_utf8(buffer)
        .map_err(|_| KernelError::Inval)?
        .trim())
}

// Write permission is enforced by DAC
unsafe fn sysfs_write(
    _mount: *const fs::Mount,
    inode_num: u32,
    buffer: &[u8],
    _offset: u32,
) -> KResult<usize> {
    match decode(inode_num) {
        Some(Attr::ModuleState(index)) => {
            let enabled = match parse_value(buffer)? {
                "1" | "on" | "enable" | "active" => true,
                "0" | "off" | "disable" | "destroyed" => false,
                _ => return Err(KernelError::Inval),
            };
            modules::set_builtin_enabled(index, enabled)?;
        }
        _ => return Err(KernelError::ROFS),
    }
    Ok(buffer.len())
}

unsafe fn sysfs_truncate(_mount: *const fs::Mount, inode_num: u32) -> KResult<()> {
    // Attributes have no stored data; O_TRUNC on a writable one is a no-op.
    match decode(inode_num) {
        Some(Attr::ModuleState(_)) => Ok(()),
        _ => Err(KernelError::ROFS),
    }
}

pub unsafe fn sysfs_read(
    mount: *const fs::Mount,
    inode_num: u32,
    buffer: &mut [u8],
    offset: u32,
) -> KResult<usize> {
    let attr = decode(inode_num).ok_or(KernelError::Inval)?;
    fs::seq::read(mount, inode_num, buffer, offset, &mut |buf| {
        generate(attr, buf)
    })
}

fn generate(attr: Attr, buf: &mut [u8]) -> KResult<usize> {
    match attr {
        Attr::MemStat => Ok(generate_mem_stat(buf)),
        Attr::MemDebug => Ok(generate_mem_debug(buf)),
        Attr::Initcalls => Ok(generate_initcalls(buf)),
        Attr::SysInfo => Ok(generate_system_info(buf)),
        _ => generate_value(attr, buf),
    }
}

fn generate_value(attr: Attr, buf: &mut [u8]) -> KResult<usize> {
    let mut writer = BufferWriter::new(buf);
    match attr {
        Attr::MemStat | Attr::MemDebug | Attr::Initcalls | Attr::SysInfo => {
            return Err(KernelError::Inval);
        }
        Attr::CpuOnline => write_cpu_list(&mut writer, smp::cpu::is_online),
        Attr::CpuPossible => write_cpu_list(&mut writer, |_| true),
        Attr::CpuPresent => write_cpu_list(&mut writer, |cpu| cpu < smp::cpu::cpu_count()),
        Attr::ModuleState(index) => {
            let _ = writeln!(writer, "{}", module_state_name(index));
        }
        Attr::BlockSize(device) => {
            let device = drivers::device_at(device).ok_or(KernelError::NoDev)?;
            // Always in 512-byte units, like Linux.
            let sectors = device.sector_count as u64 * device.sector_size as u64 / 512;
            let _ = writeln!(writer, "{}", sectors);
        }
        Attr::BlockStart(device) => {
            let device = drivers::device_at(device).ok_or(KernelError::NoDev)?;
            let _ = writeln!(writer, "{}", device.start_lba);
        }
    }
    Ok(writer.written())
}

fn write_cpu_list(writer: &mut BufferWriter, include: impl Fn(usize) -> bool) {
    let mut first = true;
    let mut cpu = 0;
    while cpu < smp::MAX_CPUS {
        if !include(cpu) {
            cpu += 1;
            continue;
        }
        let start = cpu;
        while cpu + 1 < smp::MAX_CPUS && include(cpu + 1) {
            cpu += 1;
        }
        let separator = if first { "" } else { "," };
        let _ = if start == cpu {
            write!(writer, "{}{}", separator, start)
        } else {
            write!(writer, "{}{}-{}", separator, start, cpu)
        };
        first = false;
        cpu += 1;
    }
    let _ = writeln!(writer);
}

fn module_state_name(index: usize) -> &'static str {
    match modules::builtin_state(index) {
        None => "unloaded",
        Some(ModuleState::Registered) => "registered",
        Some(ModuleState::Creating | ModuleState::Initializing) => "coming",
        Some(ModuleState::Active) => "active",
        Some(ModuleState::Destroying) => "going",
        Some(ModuleState::Destroyed) => "destroyed",
    }
}

fn generate_mem_stat(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);
    dump::print_memstat(|args| {
        let _ = writer.write_fmt(*args);
    });
    writer.written()
}

fn generate_mem_debug(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);
    dump::print_memdebug(|args| {
        let _ = writer.write_fmt(*args);
    });
    writer.written()
}

fn generate_initcalls(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);
    if let Some((start, end, pages)) = crate::initcall::freed_memory_info() {
        let _ = writeln!(
            writer,
            "Init memory reclaimed: {:#x} - {:#x} ({} pages)",
            start, end, pages
        );
    }
    let history = crate::initcall::boot_history();
    let _ = writeln!(writer, "Boot initcalls executed: {}", history.len());
    for record in history {
        let lvl_name = crate::initcall::InitcallLevel::from_u8(record.level)
            .map(|l| l.as_str())
            .unwrap_or("unknown");
        let _ = writeln!(
            writer,
            "  [{}] {:<32} -> ret={}",
            lvl_name, record.name, record.result
        );
    }
    writer.written()
}

fn generate_system_info(buf: &mut [u8]) -> usize {
    let mut writer = BufferWriter::new(buf);
    let _ = writeln!(writer, "System: KFS (Kernel From Scratch)");
    let _ = writeln!(writer, "Arch: i686 (32-bit x86 protected mode)");
    let _ = writeln!(
        writer,
        "CPUs: {}/{} online",
        crate::smp::cpu::online_count(),
        crate::smp::MAX_CPUS
    );
    let _ = writeln!(writer, "Ticks: {}", crate::interrupts::timer::get_ticks());
    writer.written()
}

unsafe fn create_sysfs_child(
    parent: *mut VfsNode,
    master: *mut VfsNode,
    name: &str,
    inode: u32,
    node_type: VfsNodeType,
    rights: u16,
) -> KResult<*mut VfsNode> {
    let node = fs::alloc_vfs_node()?;
    core::ptr::write_bytes((*node).name.as_mut_ptr(), 0, 256);
    let name_bytes = name.as_bytes();
    let to_copy = name_bytes.len().min(255);
    core::ptr::copy_nonoverlapping(name_bytes.as_ptr(), (*node).name.as_mut_ptr(), to_copy);
    (*node).inode = inode;
    (*node).node_type = node_type;
    (*node).rights = rights;
    (*node).owner_uid = 0;
    (*node).owner_gid = 0;
    (*node).size = 0;
    (*node).links = 1;
    (*node).master = master;
    (*node).father = parent;
    (*node).children = core::ptr::null_mut();
    (*node).next_of_kin = core::ptr::null_mut();
    (*node).mount = core::ptr::null();
    fs::append_child(parent, node);
    Ok(node)
}

unsafe fn dir(
    parent: *mut VfsNode,
    master: *mut VfsNode,
    name: &str,
    inode: u32,
) -> KResult<*mut VfsNode> {
    create_sysfs_child(parent, master, name, inode, VfsNodeType::Directory, 0o555)
}

unsafe fn attr(
    parent: *mut VfsNode,
    master: *mut VfsNode,
    name: &str,
    inode: u32,
    mode: u16,
) -> KResult<()> {
    create_sysfs_child(parent, master, name, inode, VfsNodeType::File, mode).map(|_| ())
}

pub unsafe fn populate_sysfs_root(target: *mut VfsNode, master: *mut VfsNode) -> KResult<()> {
    use super::*;

    let kernel_dir = dir(target, master, "kernel", SYSFS_KERNEL_DIR_INODE)?;
    let debug_dir = dir(kernel_dir, master, "debug", SYSFS_DEBUG_DIR_INODE)?;
    attr(
        debug_dir,
        master,
        "memstat",
        SYSFS_DEBUG_MEMSTAT_INODE,
        0o444,
    )?;
    attr(
        debug_dir,
        master,
        "memdebug",
        SYSFS_DEBUG_MEMDEBUG_INODE,
        0o444,
    )?;
    attr(
        debug_dir,
        master,
        "initcalls",
        SYSFS_DEBUG_INITCALLS_INODE,
        0o444,
    )?;
    attr(
        debug_dir,
        master,
        "sysinfo",
        SYSFS_DEBUG_SYSINFO_INODE,
        0o444,
    )?;

    let devices_dir = dir(target, master, "devices", SYSFS_DEVICES_DIR_INODE)?;
    let system_dir = dir(devices_dir, master, "system", SYSFS_SYSTEM_DIR_INODE)?;
    let cpu_dir = dir(system_dir, master, "cpu", SYSFS_CPU_DIR_INODE)?;
    attr(cpu_dir, master, "online", SYSFS_CPU_ONLINE_INODE, 0o444)?;
    attr(cpu_dir, master, "possible", SYSFS_CPU_POSSIBLE_INODE, 0o444)?;
    attr(cpu_dir, master, "present", SYSFS_CPU_PRESENT_INODE, 0o444)?;

    let block_dir = dir(target, master, "block", SYSFS_BLOCK_DIR_INODE)?;
    let mut disk_dirs: [*mut VfsNode; drivers::MAX_BLOCK_DEVICES] =
        [core::ptr::null_mut(); drivers::MAX_BLOCK_DEVICES];
    for pass_partitions in [false, true] {
        for device_id in 0..drivers::MAX_BLOCK_DEVICES {
            let Some(device) = drivers::device_at(device_id) else {
                continue;
            };
            if device.partition != pass_partitions {
                continue;
            }
            let parent = if device.partition {
                match disk_dirs.get(device.parent) {
                    Some(&parent) if !parent.is_null() => parent,
                    _ => continue,
                }
            } else {
                block_dir
            };
            let name_len = device
                .name
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(device.name.len());
            let Ok(name) = core::str::from_utf8(&device.name[..name_len]) else {
                continue;
            };
            let base = SYSFS_BLOCK_BASE + device_id as u32 * SYSFS_BLOCK_STRIDE;
            let node = dir(parent, master, name, base)?;
            attr(node, master, "size", base + SYSFS_BLOCK_SIZE, 0o444)?;
            if device.partition {
                attr(node, master, "start", base + SYSFS_BLOCK_START, 0o444)?;
            } else {
                disk_dirs[device_id] = node;
            }
        }
    }

    let module_dir = dir(target, master, "module", SYSFS_MODULE_DIR_INODE)?;
    for index in 0..modules::builtin_count() {
        let Some(name) = modules::builtin_name(index) else {
            continue;
        };
        let base = SYSFS_MODULE_BASE + index as u32 * SYSFS_MODULE_STRIDE;
        let node = dir(module_dir, master, name, base)?;
        attr(node, master, "state", base + SYSFS_MODULE_STATE, 0o644)?;
    }

    Ok(())
}

pub unsafe fn sysfs_lazy_load_directory(
    _mount: *const fs::Mount,
    _dir_node: *mut VfsNode,
    _dir_inode_num: u32,
) -> KResult<()> {
    // Directories are pre-populated by populate_sysfs_root
    Ok(())
}

unsafe fn sysfs_unlink(_mount: *const fs::Mount, _parent_inode: u32, _name: &str) -> KResult<()> {
    Err(KernelError::ROFS)
}

unsafe fn sysfs_evict(_mount: *const fs::Mount, _inode_num: u32) -> KResult<()> {
    Ok(())
}

pub const SYSFS_BACKEND: FsBackend = FsBackend {
    create: sysfs_create,
    read: sysfs_read,
    write: sysfs_write,
    truncate: sysfs_truncate,
    lazy_load_directory: sysfs_lazy_load_directory,
    unlink: sysfs_unlink,
    evict: sysfs_evict,
};
