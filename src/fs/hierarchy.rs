use crate::{
    error::{KResult, KernelError},
    fs::{self, VfsNode, VfsNodeType},
    pr_err,
};

pub struct DirEntrySpec {
    pub path: &'static str,
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
}

pub struct FileEntrySpec {
    pub path: &'static str,
    pub mode: u16,
}

/// Directories that are persistent parts of the root image.
pub static KERNEL_DIRECTORIES: &[DirEntrySpec] = &[
    DirEntrySpec { path: "/bin", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/boot", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/etc", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/home", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/lib", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/media", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/mnt", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/opt", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/root", mode: 0o700, uid: 0, gid: 0 },
    DirEntrySpec { path: "/run", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/sbin", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/srv", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/tmp", mode: 0o1777, uid: 0, gid: 0 },
    DirEntrySpec { path: "/usr", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/dev", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/var", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/usr/bin", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/usr/sbin", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/usr/lib", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/usr/share", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/var/cache", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/var/lib", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/var/log", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/var/spool", mode: 0o755, uid: 0, gid: 0 },
    DirEntrySpec { path: "/var/tmp", mode: 0o1777, uid: 0, gid: 0 },
];

/// Runtime mount points are image directories whose contents are kernel-owned.
pub static RUNTIME_MOUNT_POINTS: &[&str] = &["/dev", "/proc", "/sys", "/run"];
pub static PERSISTENT_FILES: &[FileEntrySpec] = &[
    FileEntrySpec { path: "/etc/shadow", mode: 0o600 },
    FileEntrySpec { path: "/var/log/kernel.log", mode: 0o644 },
];

pub unsafe fn validate_directory(path: &str) -> KResult<*mut VfsNode> {
    if path.is_empty() || !path.starts_with('/') {
        return Err(KernelError::EINVAL);
    }
    let mut current = fs::ROOT_NODE;
    if current.is_null() {
        return Err(KernelError::EINVAL);
    }

    for segment in path.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if (*current).node_type != VfsNodeType::Directory {
            return Err(KernelError::ENOTDIR);
        }
        if segment == ".." {
            if !(*current).father.is_null() {
                current = (*current).father;
            }
            continue;
        }

        let mut child = (*current).children;
        let mut found: *mut VfsNode = core::ptr::null_mut();

        while !child.is_null() {
            let name_len = (*child).name.iter().position(|&c| c == 0).unwrap_or(256);
            let child_name = core::str::from_utf8(&(&(*child).name)[..name_len]).unwrap_or("");
            if child_name == segment {
                found = child;
                break;
            }
            child = (*child).next_of_kin;
        }

        if found.is_null()
            && (*current).node_type == VfsNodeType::Directory
            && (*current).children.is_null()
            && (*current).inode != 0
        {
            (*current).lazy_load_directory()?;
            let mut child = (*current).children;
            while !child.is_null() {
                let name_len = (*child).name.iter().position(|&c| c == 0).unwrap_or(256);
                let child_name = core::str::from_utf8(&(&(*child).name)[..name_len]).unwrap_or("");
                if child_name == segment {
                    found = child;
                    break;
                }
                child = (*child).next_of_kin;
            }
        }

        if found.is_null() {
            pr_err!("root image is missing required path: {}\n", path);
            return Err(KernelError::ENOENT);
        }
        current = found;
    }

    if (*current).node_type != VfsNodeType::Directory {
        return Err(KernelError::ENOTDIR);
    }

    Ok(current)
}

pub unsafe fn create_directory(path: &str, mode: u16, uid: u32, gid: u32) -> KResult<*mut VfsNode> {
    let parent_end = path.rfind('/').ok_or(KernelError::EINVAL)?;
    let name = &path[parent_end + 1..];
    let parent = if parent_end == 0 {
        fs::ROOT_NODE
    } else {
        validate_directory(&path[..parent_end])?
    };
    let node = fs::create_child_node(parent, name, VfsNodeType::Directory, mode)?;
    (*node).owner_uid = uid;
    (*node).owner_gid = gid;
    Ok(node)
}

#[unsafe(link_section = ".init.text")]
pub unsafe fn init_unix_hierarchy() -> KResult<()> {
    let root = fs::ROOT_NODE;
    if root.is_null() {
        pr_err!("init_unix_hierarchy: ROOT_NODE is null\n");
        return Err(KernelError::EINVAL);
    }

    for dir in KERNEL_DIRECTORIES {
        let node = validate_directory(dir.path)?;
        if (*node).rights != dir.mode {
            pr_err!(
                "root image path {} has mode {:o}, expected {:o}\n",
                dir.path,
                (*node).rights,
                dir.mode
            );
            return Err(KernelError::EACCES);
        }
    }
    for path in RUNTIME_MOUNT_POINTS {
        validate_directory(path)?;
    }
    for file in PERSISTENT_FILES {
        let node = fs::resolve_path(file.path, root).map_err(|error| {
            pr_err!("root image is missing required file: {}\n", file.path);
            error
        })?;
        if (*node).node_type != VfsNodeType::File {
            pr_err!("root image path {} is not a regular file\n", file.path);
            return Err(KernelError::EISDIR);
        }
        if (*node).rights != file.mode {
            pr_err!(
                "root image file {} has mode {:o}, expected {:o}\n",
                file.path,
                (*node).rights,
                file.mode
            );
            return Err(KernelError::EACCES);
        }
    }

    fs::procfs::mount_procfs()?;
    fs::sysfs::mount_sysfs()?;
    fs::mark_persistent_hierarchy_ready();
    fs::mark_runtime_filesystems_ready();

    fs::print_vfs_tree(fs::ROOT_NODE, 0);
    Ok(())
}
