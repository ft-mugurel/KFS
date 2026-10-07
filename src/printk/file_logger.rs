use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::{
    error::{KResult, KernelError},
    fs::{self, VfsNodeType},
    pr_info, pr_warn,
};

static FLUSHED_POS: AtomicUsize = AtomicUsize::new(0);
static FLUSH_LOCK: AtomicBool = AtomicBool::new(false);

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

        let result = (|| -> KResult<usize> {
            let log_file = fs::resolve_path("/var/log/kernel.log", root)?;

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
