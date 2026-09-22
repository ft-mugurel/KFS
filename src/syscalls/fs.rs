use crate::error::KernelError;
use crate::fs::{self, VfsNodeType};
use crate::sched::{self, ContextFrame};
use crate::utils;
use crate::security::{self, Decision, Operation};

fn parse_parent_path(path: &str) -> Result<(&str, &str), KernelError> {
    if path.is_empty() || path == "/" {
        return Err(KernelError::EINVAL);
    }

    if let Some((parent, name)) = path.rsplit_once('/') {
        let parent_path = if parent.is_empty() { "/" } else { parent };
        if name.is_empty() {
            return Err(KernelError::EINVAL);
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
        _ => Err(KernelError::EINVAL),
    }
}

unsafe fn lookup_parent(
    path: &str,
    cwd: *mut fs::VfsNode,
) -> Result<*mut fs::VfsNode, KernelError> {
    if path.is_empty() {
        return Ok(cwd);
    }

    fs::resolve_path(path, cwd)
}

unsafe fn find_child(mut node: *mut fs::VfsNode, name: &str) -> *mut fs::VfsNode {
    while !node.is_null() {
        let name_len = (*node)
            .name
            .iter()
            .position(|&c| c == 0)
            .unwrap_or((*node).name.len());
        if core::str::from_utf8(&(*node).name[..name_len]).unwrap_or("") == name {
            return node;
        }
        node = (*node).next_of_kin;
    }
    core::ptr::null_mut()
}

pub unsafe fn syscall_mknod(regs: *mut ContextFrame) {
    let path_ptr = (*regs).arg1() as *const u8;
    let mode = (*regs).arg2();
    let path = utils::c_str_to_rust(path_ptr);
    let task = sched::current().as_mut().unwrap();

    let (parent_path, name) = match parse_parent_path(path) {
        Ok(parts) => parts,
        Err(err) => {
            (*regs).set_return_error(err);
            return;
        }
    };

    let parent = match lookup_parent(parent_path, task.cwd) {
        Ok(node) => node,
        Err(err) => {
            (*regs).set_return_error(err);
            return;
        }
    };

    if parent.is_null() || (*parent).node_type != VfsNodeType::Directory {
        (*regs).set_return_error(KernelError::ENOTDIR);
        return;
    }

    if security::check(
        &task.credentials,
        &security::object_for_node(parent),
        Operation::Create,
    ) == Decision::Deny
    {
        (*regs).set_return_error(KernelError::EACCES);
        return;
    }

    if !find_child((*parent).children, name).is_null() {
        (*regs).set_return_error(KernelError::EEXIST);
        return;
    }

    let node_type = match mode_to_node_type(mode) {
        Ok(kind) => kind,
        Err(err) => {
            (*regs).set_return_error(err);
            return;
        }
    };

    match fs::create_child_node(parent, name, node_type, (mode & 0x0FFF) as u16) {
        Ok(node) => {
            if node_type == VfsNodeType::Directory {
                (*node).master = node;
            }
            (*regs).set_return_value(0);
        }
        Err(err) => (*regs).set_return_error(err),
    }
}

unsafe fn reparent_children(parent: *mut fs::VfsNode, new_parent: *mut fs::VfsNode) {
    let mut child = (*parent).children;
    while !child.is_null() {
        (*child).father = new_parent;
        child = (*child).next_of_kin;
    }
}

pub unsafe fn syscall_mount(regs: *mut ContextFrame) {
    let source_ptr = (*regs).arg1() as *const u8;
    let target_ptr = (*regs).arg2() as *const u8;
    let source_path = utils::c_str_to_rust(source_ptr);
    let target_path = utils::c_str_to_rust(target_ptr);
    let task = sched::current().as_mut().unwrap();

    if security::check(&task.credentials, &security::SecurityObject::System, Operation::Mount)
        == Decision::Deny
    {
        (*regs).set_return_error(KernelError::EPERM);
        return;
    }

    let source = match fs::resolve_path(source_path, task.cwd) {
        Ok(node) => node,
        Err(err) => {
            (*regs).set_return_error(err);
            return;
        }
    };

    let target = match fs::resolve_path(target_path, task.cwd) {
        Ok(node) => node,
        Err(err) => {
            (*regs).set_return_error(err);
            return;
        }
    };

    if (*source).node_type != VfsNodeType::Directory
        || (*target).node_type != VfsNodeType::Directory
    {
        (*regs).set_return_error(KernelError::ENOTDIR);
        return;
    }

    if source == target {
        (*regs).set_return_error(KernelError::EINVAL);
        return;
    }

    if !(*target).children.is_null() {
        (*regs).set_return_error(KernelError::EBUSY);
        return;
    }

    (*target).children = (*source).children;
    reparent_children(source, target);
    (*source).children = core::ptr::null_mut();

    if let Err(err) = fs::mount_node(target, source) {
        (*regs).set_return_error(err);
        return;
    }

    (*regs).set_return_value(0);
}

#[inline(always)]
unsafe fn current_cwd() -> *mut fs::VfsNode {
    (*sched::current().as_mut().unwrap()).cwd
}

pub unsafe fn syscall_umount(regs: *mut ContextFrame) {
    let target_ptr = (*regs).arg1() as *const u8;
    let target_path = utils::c_str_to_rust(target_ptr);

    let credentials = &sched::current().as_ref().unwrap().credentials;
    if security::check(credentials, &security::SecurityObject::System, Operation::Unmount)
        == Decision::Deny
    {
        (*regs).set_return_error(KernelError::EPERM);
        return;
    }

    let target = match fs::resolve_path(target_path, current_cwd()) {
        Ok(node) => node,
        Err(err) => {
            (*regs).set_return_error(err);
            return;
        }
    };

    if (*target).node_type != VfsNodeType::Directory {
        (*regs).set_return_error(KernelError::ENOTDIR);
        return;
    }

    let source = (*target).master;
    if source.is_null() || source == target {
        (*regs).set_return_error(KernelError::EINVAL);
        return;
    }

    (*source).children = (*target).children;
    reparent_children(target, source);
    (*target).children = core::ptr::null_mut();

    if let Err(err) = fs::umount_node(target) {
        (*regs).set_return_error(err);
        return;
    }
    (*regs).set_return_value(0);
}
