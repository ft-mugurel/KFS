use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::{
    error::{KResult, KernelError},
    fs::{self, VfsNode, VfsNodeType},
    pr_info, pr_warn,
};

static FLUSHED_POS: AtomicUsize = AtomicUsize::new(0);
static FLUSH_LOCK: AtomicBool = AtomicBool::new(false);
static ROTATED_THIS_BOOT: AtomicBool = AtomicBool::new(false);

const MAX_LOG_ROTATIONS: usize = 3;
const ROTATED_NAMES: [&str; MAX_LOG_ROTATIONS] = [
    "kernel.log.1",
    "kernel.log.2",
    "kernel.log.3",
];
const ROTATED_PATHS: [&str; MAX_LOG_ROTATIONS] = [
    "/var/log/kernel.log.1",
    "/var/log/kernel.log.2",
    "/var/log/kernel.log.3",
];

unsafe fn copy_file_content(src: *mut VfsNode, dst: *mut VfsNode) -> KResult<()> {
    (*dst).truncate()?;
    let mut offset = 0u32;
    let mut buf = [0u8; 1024];
    let total_size = (*src).size;
    while offset < total_size {
        let to_read = (total_size - offset).min(buf.len() as u32) as usize;
        let read_bytes = (*src).read(&mut buf[..to_read], offset)?;
        if read_bytes == 0 {
            break;
        }
        let written = (*dst).write(&buf[..read_bytes], offset)?;
        offset += written as u32;
        if written < read_bytes {
            break;
        }
    }
    Ok(())
}

unsafe fn get_or_create_child_file(
    parent: *mut VfsNode,
    name: &str,
    full_path: &str,
    root: *mut VfsNode,
) -> KResult<*mut VfsNode> {
    match fs::resolve_path(full_path, root) {
        Ok(node) => {
            if (*node).node_type == VfsNodeType::File {
                Ok(node)
            } else {
                Err(KernelError::EINVAL)
            }
        }
        Err(_) => fs::create_child_node(parent, name, VfsNodeType::File, 0o644),
    }
}

pub unsafe fn rotate_boot_logs() -> KResult<()> {
    if ROTATED_THIS_BOOT.swap(true, Ordering::SeqCst) {
        return Ok(());
    }

    let root = fs::ROOT_NODE;
    if root.is_null() {
        return Ok(());
    }

    let var_log = match fs::resolve_path("/var/log", root) {
        Ok(dir) => dir,
        Err(_) => return Ok(()),
    };

    if (*var_log).node_type != VfsNodeType::Directory {
        return Ok(());
    }

    let current_log = match fs::resolve_path("/var/log/kernel.log", root) {
        Ok(file) => file,
        Err(_) => return Ok(()),
    };

    if (*current_log).node_type != VfsNodeType::File {
        return Ok(());
    }

    let prev_size = (*current_log).size;
    if prev_size == 0 {
        return Ok(());
    }

    // Rotate older archives backwards: kernel.log.2 -> kernel.log.3, kernel.log.1 -> kernel.log.2
    for idx in (1..MAX_LOG_ROTATIONS).rev() {
        let src_path = ROTATED_PATHS[idx - 1];
        if let Ok(src_node) = fs::resolve_path(src_path, root) {
            if (*src_node).node_type == VfsNodeType::File && (*src_node).size > 0 {
                let dst_name = ROTATED_NAMES[idx];
                let dst_path = ROTATED_PATHS[idx];
                if let Ok(dst_node) = get_or_create_child_file(var_log, dst_name, dst_path, root) {
                    let _ = copy_file_content(src_node, dst_node);
                }
            }
        }
    }

    // Rotate current log: kernel.log -> kernel.log.1
    let dst_name = ROTATED_NAMES[0];
    let dst_path = ROTATED_PATHS[0];
    let dst_node = get_or_create_child_file(var_log, dst_name, dst_path, root)?;
    copy_file_content(current_log, dst_node)?;

    // Truncate current log so the current boot starts with a clean file
    (*current_log).truncate()?;
    let _ = crate::fs::buffer_cache::bsync();

    pr_info!(
        "Kernel log rotated per boot: /var/log/kernel.log ({} bytes) -> /var/log/kernel.log.1\n",
        prev_size
    );

    Ok(())
}

pub fn flush_to_file() -> KResult<usize> {
    unsafe {
        let root = fs::ROOT_NODE;
        if root.is_null() {
            return Ok(0);
        }

        if FLUSH_LOCK
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Ok(0);
        }

        let _ = rotate_boot_logs();

        let result = (|| -> KResult<usize> {
            let log_file = match fs::resolve_path("/var/log/kernel.log", root) {
                Ok(file) => file,
                Err(_) => {
                    let var_log = fs::resolve_path("/var/log", root)?;
                    fs::create_child_node(var_log, "kernel.log", VfsNodeType::File, 0o644)?
                }
            };

            if (*log_file).node_type != VfsNodeType::File {
                return Err(KernelError::EINVAL);
            }

            // Ext2 singly indirect block limit is ~268 KB. Rotate/truncate if log file exceeds
            // 200 KB to avoid ENOSYS
            const MAX_LOG_FILE_SIZE: u32 = 200 * 1024;
            if (*log_file).size > MAX_LOG_FILE_SIZE {
                let _ = (*log_file).truncate();
                let head = super::klog::head();
                FLUSHED_POS.store(
                    head.saturating_sub(super::klog::KLOG_BUFFER_SIZE),
                    Ordering::Relaxed,
                );
            }

            let mut total_written = 0;
            let mut chunk = [0u8; 4096];
            let head = super::klog::head();
            let mut current = FLUSHED_POS.load(Ordering::Relaxed);

            while current < head {
                let to_read = (head - current).min(chunk.len());
                let bytes_read = super::klog::read_range(current, &mut chunk[..to_read]);
                if bytes_read == 0 {
                    break;
                }

                let offset = (*log_file).size;
                let written = match (*log_file).write(&chunk[..bytes_read], offset) {
                    Ok(w) => w,
                    Err(_) => {
                        // Truncate and retry once
                        let _ = (*log_file).truncate();
                        let head = super::klog::head();
                        current = head.saturating_sub(super::klog::KLOG_BUFFER_SIZE);
                        match (*log_file).write(&chunk[..bytes_read], 0) {
                            Ok(w) => w,
                            Err(_) => break,
                        }
                    }
                };
                current += written;
                total_written += written;

                if written < bytes_read {
                    break;
                }
            }

            FLUSHED_POS.store(current, Ordering::Relaxed);

            let _ = crate::fs::buffer_cache::bsync();

            Ok(total_written)
        })();

        FLUSH_LOCK.store(false, Ordering::Release);
        result
    }
}

#[unsafe(link_section = ".init.text")]
pub fn init_file_logger() -> KResult<()> {
    unsafe {
        if let Err(e) = rotate_boot_logs() {
            pr_warn!("Kernel log boot rotation failed: {:?}\n", e);
        }
    }
    match flush_to_file() {
        Ok(bytes) => {
            pr_info!(
                "Kernel log persistence initialized: /var/log/kernel.log ({} bytes written)\n",
                bytes
            );
        }
        Err(e) => {
            pr_warn!("Kernel log persistence initialization failed: {:?}\n", e);
        }
    }
    Ok(())
}

crate::late_initcall!(init_file_logger);
