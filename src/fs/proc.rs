use core::fmt::Write;

use crate::{
    error::{KResult, KernelError},
    fs::{self, VfsNodeType},
    interrupts::timer,
    paging, smp,
};

pub const PROC_MEMINFO_INODE: u32 = 0xF000_0001;
pub const PROC_CPUINFO_INODE: u32 = 0xF000_0002;
pub const PROC_UPTIME_INODE: u32 = 0xF000_0003;
pub const PROC_MOUNTS_INODE: u32 = 0xF000_0004;

pub fn init_procfs() -> KResult<()> {
    unsafe {
        let root = fs::ROOT_NODE;
        if root.is_null() {
            return Err(KernelError::EINVAL);
        }

        let target = match fs::resolve_path("/proc", root) {
            Ok(n) => n,
            Err(_) => {
                // Try to create /proc if it doesn't exist
                let proc_node = fs::alloc_vfs_node()?;
                core::ptr::write_bytes((*proc_node).name.as_mut_ptr(), 0, 256);
                core::ptr::copy_nonoverlapping(b"proc".as_ptr(), (*proc_node).name.as_mut_ptr(), 4);
                (*proc_node).node_type = VfsNodeType::Directory;
                (*proc_node).rights = 0o555;
                (*proc_node).father = root;
                (*proc_node).next_of_kin = (*root).children;
                (*root).children = proc_node;
                proc_node
            }
        };

        // Helper to create a proc file
        let create_proc_file = |name: &[u8], inode: u32| -> KResult<()> {
            let node = fs::alloc_vfs_node()?;
            core::ptr::write_bytes((*node).name.as_mut_ptr(), 0, 256);
            core::ptr::copy_nonoverlapping(name.as_ptr(), (*node).name.as_mut_ptr(), name.len());
            (*node).inode = inode;
            (*node).node_type = VfsNodeType::File;
            (*node).rights = 0o444;
            (*node).size = 0;
            (*node).father = target;
            (*node).next_of_kin = (*target).children;
            (*target).children = node;
            Ok(())
        };

        create_proc_file(b"meminfo", PROC_MEMINFO_INODE)?;
        create_proc_file(b"cpuinfo", PROC_CPUINFO_INODE)?;
        create_proc_file(b"uptime", PROC_UPTIME_INODE)?;
        create_proc_file(b"mounts", PROC_MOUNTS_INODE)?;
    }

    Ok(())
}

crate::late_initcall!(init_procfs);

struct BufWriter<'a> {
    buf: &'a mut [u8],
    offset: usize,
}
impl<'a> Write for BufWriter<'a> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let rem = self.buf.len().saturating_sub(self.offset);
        let copy_len = rem.min(bytes.len());
        if copy_len > 0 {
            self.buf[self.offset..self.offset + copy_len].copy_from_slice(&bytes[..copy_len]);
            self.offset += copy_len;
        }
        Ok(())
    }
}

pub unsafe fn proc_read(inode: u32, buffer: &mut [u8], offset: u32) -> KResult<usize> {
    let mut tmp = [0u8; 1024];
    let len = match inode {
        PROC_MEMINFO_INODE => generate_meminfo(&mut tmp),
        PROC_CPUINFO_INODE => generate_cpuinfo(&mut tmp),
        PROC_UPTIME_INODE => generate_uptime(&mut tmp),
        PROC_MOUNTS_INODE => generate_mounts(&mut tmp),
        _ => return Err(KernelError::ENOENT),
    };

    if offset as usize >= len {
        return Ok(0);
    }
    let remaining = len - offset as usize;
    let to_copy = remaining.min(buffer.len());
    buffer[..to_copy].copy_from_slice(&tmp[offset as usize..offset as usize + to_copy]);
    Ok(to_copy)
}

fn generate_meminfo(buf: &mut [u8]) -> usize {
    let mut writer = BufWriter { buf, offset: 0 };
    let kheap_stats = paging::kernel_heap_debug_stats();
    let vmem_stats = paging::vmem_debug_stats();

    let _ = write!(
        &mut writer,
        "Kernel Heap Free: {}\n",
        kheap_stats.free_bytes
    );
    let _ = write!(
        &mut writer,
        "Kernel Heap Total: {}\n",
        kheap_stats.chunk_bytes
    );
    let _ = write!(&mut writer, "VMem Free: {}\n", vmem_stats.free_bytes);
    let _ = write!(&mut writer, "VMem Total: {}\n", vmem_stats.total_bytes);
    writer.offset
}

fn generate_cpuinfo(buf: &mut [u8]) -> usize {
    let mut writer = BufWriter { buf, offset: 0 };
    let count = smp::cpu::cpu_count();
    for i in 0..count {
        let online = smp::cpu::is_online(i);
        let _ = write!(
            &mut writer,
            "processor\t: {}\nstatus\t\t: {}\n\n",
            i,
            if online { "online" } else { "offline" }
        );
    }
    writer.offset
}

fn generate_uptime(buf: &mut [u8]) -> usize {
    let mut writer = BufWriter { buf, offset: 0 };
    let ticks = timer::get_ticks();
    let _ = write!(&mut writer, "{}\n", ticks);
    writer.offset
}

fn generate_mounts(buf: &mut [u8]) -> usize {
    let mut writer = BufWriter { buf, offset: 0 };
    fs::for_each_mount(|mount| {
        let _ = write!(&mut writer, "device {} mounted\n", mount.device);
    });
    writer.offset
}
