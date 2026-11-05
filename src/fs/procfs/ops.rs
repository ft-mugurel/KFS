use core::sync::atomic::{AtomicPtr, Ordering};

use crate::{
    error::{KResult, KernelError},
    fs::{self, FsBackend, VfsNode, VfsNodeType},
    locks::Spinlock,
    sched,
};

/// Serializes changes to /proc's child list and PID directory contents
/// Lock order = (PROCESS_TABLE -> PROCFS_LOCK -> VFS_NODE_LOCK)
static PROCFS_LOCK: Spinlock<ReapedDirs> = Spinlock::new(ReapedDirs::new());

/// Set once at mount. `process_reaped` runs with PROCESS_TABLE held, so it
/// can't go through `resolve_path`.
static PROC_ROOT: AtomicPtr<VfsNode> = AtomicPtr::new(core::ptr::null_mut());

pub(super) fn set_proc_root(root: *mut VfsNode) {
    PROC_ROOT.store(root, Ordering::Release);
}

const MAX_REAPED_DIRS: usize = sched::MAX_PROCESSES;

/// PID directories already unlinked from /proc whose nodes are waiting to be
/// returned to the VFS pool once no open file or cwd references them.
struct ReapedDirs {
    dirs: [*mut VfsNode; MAX_REAPED_DIRS],
    count: usize,
}

unsafe impl Send for ReapedDirs {}

impl ReapedDirs {
    const fn new() -> Self {
        Self {
            dirs: [core::ptr::null_mut(); MAX_REAPED_DIRS],
            count: 0,
        }
    }
}

use super::{process, system};

unsafe fn procfs_create(
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

// Write permission is enforced by DAC at open(). /proc/sys files are root:root 0644.
unsafe fn procfs_write(
    _mount: *const fs::Mount,
    inode_num: u32,
    buffer: &[u8],
    _offset: u32,
) -> KResult<usize> {
    match inode_num {
        super::PROCFS_SYS_PRINTK_INODE => {
            system::write_printk(buffer)?;
            Ok(buffer.len())
        }
        _ => Err(KernelError::ROFS),
    }
}

unsafe fn procfs_truncate(_mount: *const fs::Mount, inode_num: u32) -> KResult<()> {
    // Tunables have no stored content; O_TRUNC on a writable one is a no-op.
    match inode_num {
        super::PROCFS_SYS_PRINTK_INODE => Ok(()),
        _ => Err(KernelError::ROFS),
    }
}

pub unsafe fn procfs_read(
    mount: *const fs::Mount,
    inode_num: u32,
    buffer: &mut [u8],
    offset: u32,
) -> KResult<usize> {
    if inode_num == super::PROCFS_KMSG_INODE {
        return Ok(crate::log::klog::read_kmsg(buffer, offset));
    }
    fs::seq::read(mount, inode_num, buffer, offset, &mut |buf| {
        generate(inode_num, buf)
    })
}

unsafe fn generate(inode_num: u32, buf: &mut [u8]) -> KResult<usize> {
    Ok(match inode_num {
        super::PROCFS_VERSION_INODE => system::generate_version(buf),
        super::PROCFS_UPTIME_INODE => system::generate_uptime(buf),
        super::PROCFS_MEMINFO_INODE => system::generate_meminfo(buf),
        super::PROCFS_CPUINFO_INODE => system::generate_cpuinfo(buf),
        super::PROCFS_STAT_INODE => system::generate_stat(buf),
        super::PROCFS_PARTITIONS_INODE => system::generate_partitions(buf),
        super::PROCFS_MOUNTS_INODE => system::generate_mounts(buf),
        super::PROCFS_INTERRUPTS_INODE => system::generate_interrupts(buf),
        super::PROCFS_CMDLINE_INODE => system::generate_cmdline(buf),
        super::PROCFS_DEVICES_INODE => system::generate_devices(buf),
        super::PROCFS_LOADAVG_INODE => system::generate_loadavg(buf),
        super::PROCFS_SYS_PRINTK_INODE => system::generate_printk(buf),
        inode if inode >= super::PROC_PID_BASE => {
            let pid = (inode - super::PROC_PID_BASE) / super::PROC_PID_STRIDE;
            match (inode - super::PROC_PID_BASE) % super::PROC_PID_STRIDE {
                super::PROC_FILE_OFFSET_STATUS => process::generate_proc_status(pid, buf)?,
                super::PROC_FILE_OFFSET_CMDLINE => process::generate_proc_cmdline(pid, buf)?,
                super::PROC_FILE_OFFSET_STAT => process::generate_proc_stat(pid, buf)?,
                super::PROC_FILE_OFFSET_MAPS => process::generate_proc_maps(pid, buf)?,
                super::PROC_FILE_OFFSET_CWD => process::generate_proc_cwd(pid, buf)?,
                _ => return Err(KernelError::Inval),
            }
        }
        _ => return Err(KernelError::Inval),
    })
}

pub unsafe fn create_proc_child(
    parent: *mut VfsNode,
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
    (*node).father = parent;
    (*node).master = (*parent).master;
    (*node).children = core::ptr::null_mut();
    (*node).next_of_kin = core::ptr::null_mut();
    (*node).mount = core::ptr::null();
    fs::append_child(parent, node);
    Ok(node)
}

pub unsafe fn populate_proc_root(target: *mut VfsNode) -> KResult<()> {
    // Static system nodes
    create_proc_child(
        target,
        "version",
        super::PROCFS_VERSION_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        "uptime",
        super::PROCFS_UPTIME_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        "meminfo",
        super::PROCFS_MEMINFO_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        "cpuinfo",
        super::PROCFS_CPUINFO_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        "stat",
        super::PROCFS_STAT_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        "partitions",
        super::PROCFS_PARTITIONS_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        "mounts",
        super::PROCFS_MOUNTS_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        "interrupts",
        super::PROCFS_INTERRUPTS_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        "cmdline",
        super::PROCFS_CMDLINE_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        "devices",
        super::PROCFS_DEVICES_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        "loadavg",
        super::PROCFS_LOADAVG_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        "kmsg",
        super::PROCFS_KMSG_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;

    let sys_dir = create_proc_child(
        target,
        "sys",
        super::PROCFS_SYS_DIR_INODE,
        VfsNodeType::Directory,
        0o555,
        0,
        0,
    )?;
    let sys_kernel_dir = create_proc_child(
        sys_dir,
        "kernel",
        super::PROCFS_SYS_KERNEL_DIR_INODE,
        VfsNodeType::Directory,
        0o555,
        0,
        0,
    )?;
    create_proc_child(
        sys_kernel_dir,
        "printk",
        super::PROCFS_SYS_PRINTK_INODE,
        VfsNodeType::File,
        0o644,
        0,
        0,
    )?;

    // Populate active PIDs
    populate_active_pids(target)?;
    Ok(())
}

unsafe fn populate_active_pids(target: *mut VfsNode) -> KResult<()> {
    let table = sched::PROCESS_TABLE.lock();
    let _guard = PROCFS_LOCK.lock();
    for process in table.iter().flatten() {
        create_proc_pid_entry(
            target,
            process.pid,
            process.credentials.uid,
            process.credentials.gid,
        )?;
    }
    Ok(())
}

// Caller holds PROCFS_LOCK.
unsafe fn create_proc_pid_entry(target: *mut VfsNode, pid: u32, uid: u32, gid: u32) -> KResult<()> {
    let mut pid_buf = [0u8; 12];
    let pid_str = format_u32(pid, &mut pid_buf);
    let mut child = (*target).children;
    while !child.is_null() {
        let name_len = (*child).name.iter().position(|&c| c == 0).unwrap_or(256);
        let child_name = core::str::from_utf8(&(&(*child).name)[..name_len]).unwrap_or("");
        if child_name == pid_str {
            return Ok(());
        }
        child = (*child).next_of_kin;
    }

    create_proc_child(
        target,
        pid_str,
        super::PROC_PID_BASE + (pid * super::PROC_PID_STRIDE),
        VfsNodeType::Directory,
        0o555,
        uid,
        gid,
    )?;
    Ok(())
}

/// # Safety
/// procfs must be mounted, and the caller must not hold PROCESS_TABLE,
/// OPEN_FILE_TABLE or PROCFS_LOCK (recycling reaped directories takes them).
pub unsafe fn process_created(pid: u32, uid: u32, gid: u32) -> KResult<()> {
    release_reaped_dirs();
    let proc_root = PROC_ROOT.load(Ordering::Acquire);
    if proc_root.is_null() {
        return Err(KernelError::NoEnt);
    }
    let _guard = PROCFS_LOCK.lock();
    create_proc_pid_entry(proc_root, pid, uid, gid)
}

/// Unlinks /proc/<pid>. May be called with PROCESS_TABLE held, so the nodes
/// are only queued here and freed later by release_reaped_dirs().
pub unsafe fn process_reaped(pid: u32) {
    let proc_root = PROC_ROOT.load(Ordering::Acquire);
    if proc_root.is_null() {
        return;
    }
    let mut reaped = PROCFS_LOCK.lock();
    let inode = super::PROC_PID_BASE + (pid * super::PROC_PID_STRIDE);
    let mut link: *mut *mut VfsNode = &mut (*proc_root).children;
    while !(*link).is_null() {
        let child = *link;
        if (*child).inode == inode {
            *link = (*child).next_of_kin;
            (*child).next_of_kin = core::ptr::null_mut();
            if reaped.count < MAX_REAPED_DIRS {
                let count = reaped.count;
                reaped.dirs[count] = child;
                reaped.count += 1;
            } else {
                // Queue full: leak this one rather than risk a dangling pointer.
                (*child).father = core::ptr::null_mut();
            }
            return;
        }
        link = &mut (*child).next_of_kin;
    }
}

unsafe fn is_in_dir(node: *mut VfsNode, dir: *mut VfsNode) -> bool {
    !node.is_null() && (node == dir || (*node).father == dir)
}

unsafe fn dir_in_use(dir: *mut VfsNode) -> bool {
    {
        let table = fs::OPEN_FILE_TABLE.lock();
        if table.iter().flatten().any(|file| is_in_dir(file.node, dir)) {
            return true;
        }
    }
    let table = sched::PROCESS_TABLE.lock();
    table.iter().flatten().any(|task| is_in_dir(task.cwd, dir))
}

unsafe fn release_reaped_dirs() {
    let mut pending = [core::ptr::null_mut(); MAX_REAPED_DIRS];
    let pending_count = {
        let mut reaped = PROCFS_LOCK.lock();
        let count = reaped.count;
        pending[..count].copy_from_slice(&reaped.dirs[..count]);
        reaped.count = 0;
        count
    };

    let mut still_used = [core::ptr::null_mut(); MAX_REAPED_DIRS];
    let mut still_used_count = 0;
    for &dir in &pending[..pending_count] {
        if dir_in_use(dir) {
            still_used[still_used_count] = dir;
            still_used_count += 1;
            continue;
        }
        let mut child = core::mem::take(&mut (*dir).children);
        (*dir).loaded = true; // nothing to load for a dead PID
        while !child.is_null() {
            let next = (*child).next_of_kin;
            // An orphan, so the last close frees it if a pinned open got in.
            (*child).father = core::ptr::null_mut();
            fs::defer_free(child);
            child = next;
        }
        fs::defer_free(dir);
    }

    if still_used_count > 0 {
        let mut reaped = PROCFS_LOCK.lock();
        for &dir in &still_used[..still_used_count] {
            if reaped.count < MAX_REAPED_DIRS {
                let count = reaped.count;
                reaped.dirs[count] = dir;
                reaped.count += 1;
            }
        }
    }
}

pub unsafe fn procfs_lazy_load_directory(
    _mount: *const fs::Mount,
    dir_node: *mut VfsNode,
    dir_inode_num: u32,
) -> KResult<()> {
    if dir_inode_num == super::PROCFS_ROOT_INODE {
        populate_active_pids(dir_node)?;
        return Ok(());
    }

    if dir_inode_num >= super::PROC_PID_BASE
        && (dir_inode_num - super::PROC_PID_BASE).is_multiple_of(super::PROC_PID_STRIDE)
    {
        let pid = (dir_inode_num - super::PROC_PID_BASE) / super::PROC_PID_STRIDE;
        let table = sched::PROCESS_TABLE.lock();
        let (uid, gid) = if let Some(Some(t)) = table.get(pid as usize) {
            (t.credentials.uid, t.credentials.gid)
        } else {
            (0, 0)
        };
        drop(table);

        let _guard = PROCFS_LOCK.lock();
        if !(*dir_node).children.is_null() {
            return Ok(()); // populated by another CPU meanwhile
        }

        create_proc_child(
            dir_node,
            "status",
            dir_inode_num + super::PROC_FILE_OFFSET_STATUS,
            VfsNodeType::File,
            0o444,
            uid,
            gid,
        )?;
        create_proc_child(
            dir_node,
            "cmdline",
            dir_inode_num + super::PROC_FILE_OFFSET_CMDLINE,
            VfsNodeType::File,
            0o444,
            uid,
            gid,
        )?;
        create_proc_child(
            dir_node,
            "stat",
            dir_inode_num + super::PROC_FILE_OFFSET_STAT,
            VfsNodeType::File,
            0o444,
            uid,
            gid,
        )?;
        create_proc_child(
            dir_node,
            "maps",
            dir_inode_num + super::PROC_FILE_OFFSET_MAPS,
            VfsNodeType::File,
            0o400,
            uid,
            gid,
        )?;
        create_proc_child(
            dir_node,
            "cwd",
            dir_inode_num + super::PROC_FILE_OFFSET_CWD,
            VfsNodeType::Symlink,
            0o777,
            uid,
            gid,
        )?;
    }

    Ok(())
}

fn format_u32(mut val: u32, buf: &mut [u8; 12]) -> &str {
    if val == 0 {
        buf[0] = b'0';
        return core::str::from_utf8(&buf[..1]).unwrap();
    }
    let mut i = 0;
    let mut temp = [0u8; 12];
    while val > 0 {
        temp[i] = b'0' + (val % 10) as u8;
        val /= 10;
        i += 1;
    }
    for j in 0..i {
        buf[j] = temp[i - 1 - j];
    }
    core::str::from_utf8(&buf[..i]).unwrap()
}

unsafe fn procfs_unlink(_mount: *const fs::Mount, _parent_inode: u32, _name: &str) -> KResult<()> {
    Err(KernelError::ROFS)
}

unsafe fn procfs_evict(_mount: *const fs::Mount, _inode_num: u32) -> KResult<()> {
    Ok(())
}

pub const PROCFS_BACKEND: FsBackend = FsBackend {
    create: procfs_create,
    read: procfs_read,
    write: procfs_write,
    truncate: procfs_truncate,
    lazy_load_directory: procfs_lazy_load_directory,
    unlink: procfs_unlink,
    evict: procfs_evict,
};
