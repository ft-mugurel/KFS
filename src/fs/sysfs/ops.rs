use core::fmt::Write;

use crate::{
    dump,
    error::{KResult, KernelError},
    fs::{self, FsBackend, VfsNode, VfsNodeType},
    paging,
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
    Err(KernelError::EROFS)
}

unsafe fn sysfs_write(
    _mount: *const fs::Mount,
    _inode_num: u32,
    _buffer: &[u8],
    _offset: u32,
) -> KResult<usize> {
    Err(KernelError::EROFS)
}

unsafe fn sysfs_truncate(_mount: *const fs::Mount, _inode_num: u32) -> KResult<()> {
    Err(KernelError::EROFS)
}

pub unsafe fn sysfs_read(
    _mount: *const fs::Mount,
    inode_num: u32,
    buffer: &mut [u8],
    offset: u32,
) -> KResult<usize> {
    let alloc = paging::kmalloc(4096)?;
    let temp_buf = &mut *(alloc as *mut [u8; 4096]);

    let res = (|| -> KResult<usize> {
        let total_len = match inode_num {
            super::SYSFS_MEM_STAT_INODE => generate_mem_stat(temp_buf),
            super::SYSFS_MEM_DEBUG_INODE => generate_mem_debug(temp_buf),
            super::SYSFS_INITCALLS_INODE => generate_initcalls(temp_buf),
            super::SYSFS_INFO_INODE => generate_system_info(temp_buf),
            _ => return Err(KernelError::EINVAL),
        };

        if (offset as usize) >= total_len {
            return Ok(0);
        }

        let available = total_len - offset as usize;
        let to_copy = buffer.len().min(available);
        buffer[..to_copy].copy_from_slice(&temp_buf[offset as usize..offset as usize + to_copy]);
        Ok(to_copy)
    })();

    let _ = paging::kfree(alloc);
    res
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
    let _ = writeln!(writer, "CPUs: {}", crate::smp::MAX_CPUS);
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
    uid: u32,
    gid: u32,
) -> KResult<*mut VfsNode> {
    let node = fs::alloc_vfs_node()?;
    core::ptr::write_bytes((*node).name.as_mut_ptr(), 0, 256);
    let name_bytes = name.as_bytes();
    let to_copy = name_bytes.len().min(255);
    core::ptr::copy_nonoverlapping(name_bytes.as_ptr(), (*node).name.as_mut_ptr(), to_copy);
    (*node).inode = inode;
    (*node).node_type = node_type;
    (*node).rights = rights;
    (*node).owner_uid = uid;
    (*node).owner_gid = gid;
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

pub unsafe fn populate_sysfs_root(target: *mut VfsNode, master: *mut VfsNode) -> KResult<()> {
    let kernel_dir = create_sysfs_child(
        target,
        master,
        "kernel",
        super::SYSFS_KERNEL_DIR_INODE,
        VfsNodeType::Directory,
        0o555,
        0,
        0,
    )?;

    let mem_dir = create_sysfs_child(
        kernel_dir,
        master,
        "memory",
        super::SYSFS_MEM_DIR_INODE,
        VfsNodeType::Directory,
        0o555,
        0,
        0,
    )?;

    let sys_dir = create_sysfs_child(
        kernel_dir,
        master,
        "system",
        super::SYSFS_SYS_DIR_INODE,
        VfsNodeType::Directory,
        0o555,
        0,
        0,
    )?;

    create_sysfs_child(
        mem_dir,
        master,
        "stat",
        super::SYSFS_MEM_STAT_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;

    create_sysfs_child(
        mem_dir,
        master,
        "debug",
        super::SYSFS_MEM_DEBUG_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;

    create_sysfs_child(
        sys_dir,
        master,
        "initcalls",
        super::SYSFS_INITCALLS_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;

    create_sysfs_child(
        sys_dir,
        master,
        "info",
        super::SYSFS_INFO_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;

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
    Err(KernelError::EROFS)
}

pub const SYSFS_BACKEND: FsBackend = FsBackend {
    create: sysfs_create,
    read: sysfs_read,
    write: sysfs_write,
    truncate: sysfs_truncate,
    lazy_load_directory: sysfs_lazy_load_directory,
    unlink: sysfs_unlink,
};
