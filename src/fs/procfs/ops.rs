use crate::{
    error::{KResult, KernelError},
    fs::{self, FsBackend, VfsNode, VfsNodeType},
    paging, sched,
};

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
    Err(KernelError::EROFS)
}

unsafe fn procfs_write(
    _mount: *const fs::Mount,
    _inode_num: u32,
    _buffer: &[u8],
    _offset: u32,
) -> KResult<usize> {
    Err(KernelError::EROFS)
}

unsafe fn procfs_truncate(_mount: *const fs::Mount, _inode_num: u32) -> KResult<()> {
    Err(KernelError::EROFS)
}

pub unsafe fn procfs_read(
    _mount: *const fs::Mount,
    inode_num: u32,
    buffer: &mut [u8],
    offset: u32,
) -> KResult<usize> {
    if inode_num == super::PROCFS_KMSG_INODE {
        return Ok(crate::printk::klog::read_kmsg(buffer, offset));
    }

    let alloc = paging::kmalloc(4096)?;
    let temp_buf = &mut *(alloc as *mut [u8; 4096]);

    let res = (|| -> KResult<usize> {
        let total_len = match inode_num {
            super::PROCFS_VERSION_INODE => system::generate_version(temp_buf),
            super::PROCFS_UPTIME_INODE => system::generate_uptime(temp_buf),
            super::PROCFS_MEMINFO_INODE => system::generate_meminfo(temp_buf),
            super::PROCFS_CPUINFO_INODE => system::generate_cpuinfo(temp_buf),
            super::PROCFS_STAT_INODE => system::generate_stat(temp_buf),
            super::PROCFS_PARTITIONS_INODE => system::generate_partitions(temp_buf),
            super::PROCFS_MOUNTS_INODE => system::generate_mounts(temp_buf),
            super::PROCFS_INTERRUPTS_INODE => system::generate_interrupts(temp_buf),
            super::PROCFS_CMDLINE_INODE => system::generate_cmdline(temp_buf),
            super::PROCFS_DEVICES_INODE => system::generate_devices(temp_buf),
            super::PROCFS_LOADAVG_INODE => system::generate_loadavg(temp_buf),
            inode if inode >= super::PROC_PID_BASE => {
                let pid = (inode - super::PROC_PID_BASE) / super::PROC_PID_STRIDE;
                let kind = (inode - super::PROC_PID_BASE) % super::PROC_PID_STRIDE;
                match kind {
                    super::PROC_FILE_OFFSET_STATUS => process::generate_proc_status(pid, temp_buf)?,
                    super::PROC_FILE_OFFSET_CMDLINE => {
                        process::generate_proc_cmdline(pid, temp_buf)?
                    }
                    super::PROC_FILE_OFFSET_STAT => process::generate_proc_stat(pid, temp_buf)?,
                    super::PROC_FILE_OFFSET_MAPS => process::generate_proc_maps(pid, temp_buf)?,
                    super::PROC_FILE_OFFSET_CWD => process::generate_proc_cwd(pid, temp_buf)?,
                    _ => return Err(KernelError::EINVAL),
                }
            }
            _ => return Err(KernelError::EINVAL),
        };

        if (offset as usize) >= total_len {
            return Ok(0);
        }

        let available = total_len - (offset as usize);
        let to_copy = available.min(buffer.len());
        buffer[..to_copy]
            .copy_from_slice(&temp_buf[(offset as usize)..(offset as usize + to_copy)]);
        Ok(to_copy)
    })();

    let _ = paging::kfree(alloc);
    res
}

pub unsafe fn create_proc_child(
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

pub unsafe fn populate_proc_root(target: *mut VfsNode, master: *mut VfsNode) -> KResult<()> {
    // Static system nodes
    create_proc_child(
        target,
        master,
        "version",
        super::PROCFS_VERSION_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        master,
        "uptime",
        super::PROCFS_UPTIME_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        master,
        "meminfo",
        super::PROCFS_MEMINFO_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        master,
        "cpuinfo",
        super::PROCFS_CPUINFO_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        master,
        "stat",
        super::PROCFS_STAT_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        master,
        "partitions",
        super::PROCFS_PARTITIONS_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        master,
        "mounts",
        super::PROCFS_MOUNTS_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        master,
        "interrupts",
        super::PROCFS_INTERRUPTS_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        master,
        "cmdline",
        super::PROCFS_CMDLINE_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        master,
        "devices",
        super::PROCFS_DEVICES_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        master,
        "loadavg",
        super::PROCFS_LOADAVG_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;
    create_proc_child(
        target,
        master,
        "kmsg",
        super::PROCFS_KMSG_INODE,
        VfsNodeType::File,
        0o444,
        0,
        0,
    )?;

    // Populate active PIDs
    populate_active_pids(target, master)?;
    Ok(())
}

unsafe fn populate_active_pids(target: *mut VfsNode, master: *mut VfsNode) -> KResult<()> {
    let table = sched::PROCESS_TABLE.lock();
    for process in table.iter().flatten() {
        let pid = process.pid;
        let mut pid_buf = [0u8; 12];
        let pid_str = format_u32(pid, &mut pid_buf);

        // Check if child with this name already exists
        let mut exists = false;
        let mut child = (*target).children;
        while !child.is_null() {
            let name_len = (*child).name.iter().position(|&c| c == 0).unwrap_or(256);
            let child_name = core::str::from_utf8(&(&(*child).name)[..name_len]).unwrap_or("");
            if child_name == pid_str {
                exists = true;
                break;
            }
            child = (*child).next_of_kin;
        }

        if !exists {
            create_proc_child(
                target,
                master,
                pid_str,
                super::PROC_PID_BASE + (pid * super::PROC_PID_STRIDE),
                VfsNodeType::Directory,
                0o555,
                process.credentials.uid,
                process.credentials.gid,
            )?;
        }
    }
    Ok(())
}

pub unsafe fn procfs_lazy_load_directory(
    _mount: *const fs::Mount,
    dir_node: *mut VfsNode,
    dir_inode_num: u32,
) -> KResult<()> {
    let master = (*dir_node).master;

    if dir_inode_num == super::PROCFS_ROOT_INODE {
        populate_active_pids(dir_node, master)?;
        return Ok(());
    }

    if dir_inode_num >= super::PROC_PID_BASE
        && (dir_inode_num - super::PROC_PID_BASE) % super::PROC_PID_STRIDE == 0
    {
        let pid = (dir_inode_num - super::PROC_PID_BASE) / super::PROC_PID_STRIDE;
        let table = sched::PROCESS_TABLE.lock();
        let (uid, gid) = if let Some(Some(t)) = table.get(pid as usize) {
            (t.credentials.uid, t.credentials.gid)
        } else {
            (0, 0)
        };
        drop(table);

        create_proc_child(
            dir_node,
            master,
            "status",
            dir_inode_num + super::PROC_FILE_OFFSET_STATUS,
            VfsNodeType::File,
            0o444,
            uid,
            gid,
        )?;
        create_proc_child(
            dir_node,
            master,
            "cmdline",
            dir_inode_num + super::PROC_FILE_OFFSET_CMDLINE,
            VfsNodeType::File,
            0o444,
            uid,
            gid,
        )?;
        create_proc_child(
            dir_node,
            master,
            "stat",
            dir_inode_num + super::PROC_FILE_OFFSET_STAT,
            VfsNodeType::File,
            0o444,
            uid,
            gid,
        )?;
        create_proc_child(
            dir_node,
            master,
            "maps",
            dir_inode_num + super::PROC_FILE_OFFSET_MAPS,
            VfsNodeType::File,
            0o400,
            uid,
            gid,
        )?;
        create_proc_child(
            dir_node,
            master,
            "cwd",
            dir_inode_num + super::PROC_FILE_OFFSET_CWD,
            VfsNodeType::File,
            0o444,
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
    Err(KernelError::EROFS)
}

pub const PROCFS_BACKEND: FsBackend = FsBackend {
    create: procfs_create,
    read: procfs_read,
    write: procfs_write,
    truncate: procfs_truncate,
    lazy_load_directory: procfs_lazy_load_directory,
    unlink: procfs_unlink,
};
