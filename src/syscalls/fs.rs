use kfs_abi::{
    LinuxDirent, S_IFBLK, S_IFCHR, S_IFDIR, S_IFIFO, S_IFLNK, S_IFREG, S_IFSOCK, Stat,
};

use super::args::{PATH_MAX, UserAddr, UserPtr, UserSlice, caller, syscall_define};
use crate::{
    error::KernelError,
    fs::{self, VfsNodeType},
    sched::{self, MAX_FDS_PER_PROCESS},
    security::{self, Decision, Operation},
};

fn parse_parent_path(path: &str) -> Result<(&str, &str), KernelError> {
    if path.is_empty() || path == "/" {
        return Err(KernelError::Inval);
    }

    if let Some((parent, name)) = path.rsplit_once('/') {
        let parent_path = if parent.is_empty() { "/" } else { parent };
        if name.is_empty() {
            return Err(KernelError::Inval);
        }
        Ok((parent_path, name))
    } else {
        Ok(("", path))
    }
}

fn mode_to_node_type(mode: u32) -> Result<VfsNodeType, KernelError> {
    match mode & 0xF000 {
        0x1000 => Ok(VfsNodeType::Fifo),
        0x2000 => Ok(VfsNodeType::CharDevice),
        0x4000 => Ok(VfsNodeType::Directory),
        0x6000 => Ok(VfsNodeType::BlockDevice),
        0x8000 => Ok(VfsNodeType::File),
        _ => Err(KernelError::Inval),
    }
}

unsafe fn lookup_parent(
    path: &str,
    cwd: *mut fs::VfsNode,
) -> Result<*mut fs::VfsNode, KernelError> {
    if path.is_empty() {
        return Ok(cwd);
    }

    unsafe { fs::resolve_path(path, cwd) }
}

unsafe fn find_child(mut node: *mut fs::VfsNode, name: &str) -> *mut fs::VfsNode {
    while !node.is_null() {
        let node_name = unsafe { &(*node).name };
        let name_len = node_name
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(node_name.len());
        if core::str::from_utf8(&node_name[..name_len]).unwrap_or("") == name {
            return node;
        }
        node = unsafe { (*node).next_of_kin };
    }
    core::ptr::null_mut()
}

syscall_define! {
    pub fn syscall_mknod(path: UserPtr<u8>, mode: u32) {
        let path = path.read_cstr(PATH_MAX)?;
        let caller = caller()?;
        let _pin = fs::pin_nodes();
        let (parent_path, name) = parse_parent_path(path)?;

        let parent = unsafe { lookup_parent(parent_path, caller.cwd) }?;
        if parent.is_null() || unsafe { (*parent).node_type } != VfsNodeType::Directory {
            return Err(KernelError::NotDir);
        }

        let object = unsafe { security::object_for_node(parent) };
        if security::check(&caller.credentials, &object, Operation::Create) == Decision::Deny {
            return Err(KernelError::Acces);
        }

        if !unsafe { find_child((*parent).children, name) }.is_null() {
            return Err(KernelError::Exist);
        }

        let node_type = mode_to_node_type(mode)?;
        let node = unsafe { fs::create_child_node(parent, name, node_type, (mode & 0x0FFF) as u16) }?;
        if node_type == VfsNodeType::Directory {
            unsafe { (*node).master = node };
        }
        Ok(0)
    }
}

unsafe fn reparent_children(parent: *mut fs::VfsNode, new_parent: *mut fs::VfsNode) {
    unsafe {
        let mut child = (*parent).children;
        while !child.is_null() {
            (*child).father = new_parent;
            child = (*child).next_of_kin;
        }
    }
}

syscall_define! {
    pub fn syscall_mount(source: UserPtr<u8>, target: UserPtr<u8>) {
        let source_path = source.read_cstr(PATH_MAX)?;
        let target_path = target.read_cstr(PATH_MAX)?;
        let caller = caller()?;
        let _pin = fs::pin_nodes();

        if security::check(&caller.credentials, &security::SecurityObject::System, Operation::Mount)
            == Decision::Deny
        {
            return Err(KernelError::Perm);
        }

        let source = unsafe { fs::resolve_path(source_path, caller.cwd) }?;
        let target = unsafe { fs::resolve_path(target_path, caller.cwd) }?;

        if unsafe { (*source).node_type } != VfsNodeType::Directory
            || unsafe { (*target).node_type } != VfsNodeType::Directory
        {
            return Err(KernelError::NotDir);
        }
        if source == target {
            return Err(KernelError::Inval);
        }
        if !unsafe { (*target).children }.is_null() {
            return Err(KernelError::Busy);
        }

        // Move the source's children under the target. There is no VFS tree
        // lock, so this is only safe while nothing else walks these two
        // directories (mount is root-only and rare).
        unsafe {
            (*target).children = (*source).children;
            reparent_children(source, target);
            (*source).children = core::ptr::null_mut();
            fs::mount_node(target, source)
        }?;
        Ok(0)
    }
}

syscall_define! {
    pub fn syscall_umount(target: UserPtr<u8>) {
        let target_path = target.read_cstr(PATH_MAX)?;
        let caller = caller()?;
        let _pin = fs::pin_nodes();

        if security::check(&caller.credentials, &security::SecurityObject::System, Operation::Unmount)
            == Decision::Deny
        {
            return Err(KernelError::Perm);
        }

        let target = unsafe { fs::resolve_path(target_path, caller.cwd) }?;
        if unsafe { (*target).node_type } != VfsNodeType::Directory {
            return Err(KernelError::NotDir);
        }

        let source = unsafe { (*target).master };
        if source.is_null() || source == target {
            return Err(KernelError::Inval);
        }

        // Same caveat as mount: no VFS tree lock.
        unsafe {
            (*source).children = (*target).children;
            reparent_children(target, source);
            (*target).children = core::ptr::null_mut();
            fs::umount_node(target)
        }?;
        Ok(0)
    }
}

fn file_type_bits(node_type: VfsNodeType) -> u32 {
    match node_type {
        VfsNodeType::Unknown => 0,
        VfsNodeType::Fifo => S_IFIFO,
        VfsNodeType::CharDevice => S_IFCHR,
        VfsNodeType::Directory => S_IFDIR,
        VfsNodeType::BlockDevice => S_IFBLK,
        VfsNodeType::File => S_IFREG,
        VfsNodeType::Socket => S_IFSOCK,
        VfsNodeType::Symlink => S_IFLNK,
    }
}

syscall_define! {
    /// lstat(path, buf): fills `buf` with the metadata of `path`. A final
    /// symlink is reported as itself.
    pub fn syscall_lstat(path: UserPtr<u8>, buf: UserPtr<Stat>) {
        let path = path.read_cstr(PATH_MAX)?;
        let caller = caller()?;

        let stat = {
            let _pin = fs::pin_nodes();
            let node = unsafe { &*fs::resolve_path_nofollow(path, caller.cwd)? };
            Stat {
                st_ino: node.inode,
                st_mode: file_type_bits(node.node_type) | node.rights as u32,
                st_nlink: node.links,
                st_uid: node.owner_uid,
                st_gid: node.owner_gid,
                st_size: node.size,
            }
        };
        buf.write(&stat)?;
        Ok(0)
    }
}

syscall_define! {
    /// getdents(fd, buf, count): fills `buf` with `LinuxDirent` records, each
    /// followed by the NUL-terminated name and padded to 4 bytes.
    pub fn syscall_getdents(local_fd: usize, buf: UserAddr, count: usize) {
        if buf.0 == 0 || count == 0 {
            return Err(KernelError::Inval);
        }
        let mut out = UserSlice::new(buf, count)?;
        let out = out.as_bytes_mut();
        if local_fd >= MAX_FDS_PER_PROCESS {
            return Err(KernelError::BadF);
        }
        let global_fd = unsafe { sched::current().as_ref() }.ok_or(KernelError::Srch)?.fd_tbl[local_fd]
            .ok_or(KernelError::BadF)?;
        let open_file = fs::get_open_file(global_fd).ok_or(KernelError::BadF)?;

        // The walk below can stand on a child that is being unlinked.
        let _pin = fs::pin_nodes();
        let dir = open_file.node;
        if unsafe { (*dir).node_type } != VfsNodeType::Directory {
            return Err(KernelError::NotDir);
        }
        if unsafe { (*dir).children.is_null() && (*dir).inode != 0 } {
            let _ = unsafe { (*dir).lazy_load_directory() };
        }

        let mut current_child = unsafe { (*dir).children };
        let mut skip = open_file.offset;
        while !current_child.is_null() && skip > 0 {
            current_child = unsafe { (*current_child).next_of_kin };
            skip -= 1;
        }

        let header_size = size_of::<LinuxDirent>();
        let mut bytes_written = 0usize;
        let mut index = open_file.offset;
        while !current_child.is_null() {
            let child = unsafe { &*current_child };
            let name_len = child.name.iter().position(|&c| c == 0).unwrap_or(child.name.len());
            let reclen = (header_size + name_len + 1 + 3) & !3;
            if bytes_written + reclen > count {
                break;
            }

            index += 1;
            let record = &mut out[bytes_written..bytes_written + reclen];
            record[0..4].copy_from_slice(&child.inode.to_le_bytes());
            record[4..8].copy_from_slice(&(index).to_le_bytes());
            record[8..10].copy_from_slice(&(reclen as u16).to_le_bytes());
            record[header_size..header_size + name_len].copy_from_slice(&child.name[..name_len]);
            record[header_size + name_len..].fill(0);

            bytes_written += reclen;
            fs::update_open_file_offset(global_fd, 1);
            current_child = child.next_of_kin;
        }
        Ok(bytes_written as u32)
    }
}

syscall_define! {
    pub fn syscall_unlink(path: UserPtr<u8>) {
        let path = path.read_cstr(PATH_MAX)?;
        let caller = caller()?;
        let _pin = fs::pin_nodes();
        let (parent_path, name) = parse_parent_path(path)?;

        let parent = unsafe { lookup_parent(parent_path, caller.cwd) }?;
        if parent.is_null() || unsafe { (*parent).node_type } != VfsNodeType::Directory {
            return Err(KernelError::NotDir);
        }

        let object = unsafe { security::object_for_node(parent) };
        if security::check(&caller.credentials, &object, Operation::Write) == Decision::Deny {
            return Err(KernelError::Acces);
        }

        // Resolving a path doesn't load its last directory.
        unsafe { (*parent).lazy_load_directory() }?;
        let child = unsafe { find_child((*parent).children, name) };
        if child.is_null() {
            return Err(KernelError::NoEnt);
        }

        if unsafe { (*child).node_type } == VfsNodeType::Directory {
            return Err(KernelError::IsDir);
        }

        unsafe { fs::unlink_child(parent, name, child) }?;
        Ok(0)
    }
}
