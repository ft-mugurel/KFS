use crate::sched::scheduler::{CURRENT_PID, PROCESS_TABLE};
use crate::sched::task::ContextFrame;

pub(super) unsafe fn syscall_read(regs: *mut ContextFrame) {
    unsafe {
        let fd = (*regs).arg1() as usize;
        let buf_ptr = (*regs).arg2() as *mut u8;
        let count = (*regs).arg3() as usize;
        let current_pid = CURRENT_PID;

        if fd >= crate::sched::task::MAX_FDS_PER_PROCESS {
            (*regs).set_return_value(!0u32);
            return;
        }

        let task = PROCESS_TABLE[current_pid].as_mut().unwrap();
        match task.fd_tbl[fd] {
            Some(crate::fs::vfs::FileDescriptor::TTY(_)) => {
                crate::pr_warn!("Read syscall on TTY is not yet implemented\n");
                (*regs).set_return_value(!0u32);
            }
            Some(crate::fs::vfs::FileDescriptor::Socket(sock_idx)) => {
                let mut sock = crate::ipc::SOCKETS[sock_idx].lock();

                let mut read_bytes = 0;
                let slice = core::slice::from_raw_parts_mut(buf_ptr, count);

                while read_bytes < count && sock.head != sock.tail {
                    slice[read_bytes] = sock.buffer[sock.tail];
                    sock.tail = (sock.tail + 1) % crate::ipc::SOCKET_BUFFER_SIZE;
                    read_bytes += 1;
                }
                (*regs).set_return_value(read_bytes as u32);
            }
            _ => {
                crate::pr_warn!("Unsupported fd type for read syscall\n");
                (*regs).set_return_value(!0u32);
            }
        }
    }
}

pub(super) unsafe fn syscall_write(regs: *mut ContextFrame) {
    unsafe {
        let fd_index = (*regs).arg1() as usize;
        let buf_ptr = (*regs).arg2() as *const u8;
        let count = (*regs).arg3() as usize;
        let current_pid = CURRENT_PID;

        if fd_index >= crate::sched::task::MAX_FDS_PER_PROCESS {
            (*regs).set_return_value(!0u32);
            return;
        }

        let task = PROCESS_TABLE[current_pid].as_mut().unwrap();
        let slice = core::slice::from_raw_parts(buf_ptr, count);

        match task.fd_tbl[fd_index] {
            Some(crate::fs::vfs::FileDescriptor::TTY(_)) => {
                if let Ok(s) = core::str::from_utf8(slice) {
                    crate::vga::text_mod::out::write_fmt_on(1, &format_args!("({})", current_pid));
                    crate::vga::text_mod::out::print_on(1, s);
                }
                (*regs).set_return_value(count as u32);
            }
            Some(crate::fs::vfs::FileDescriptor::Socket(sock_idx)) => {
                crate::pr_debug!("Writing {} bytes to Socket {}\n", count, sock_idx);
                let mut sock = crate::ipc::SOCKETS[sock_idx].lock();
                let mut written = 0;
                for &byte in slice {
                    let next_head = (sock.head + 1) % crate::ipc::SOCKET_BUFFER_SIZE;
                    if next_head != sock.tail {
                        let buf_idx = sock.head;
                        sock.buffer[buf_idx] = byte;
                        sock.head = next_head;
                        written += 1;
                    } else {
                        break;
                    }
                }
                (*regs).set_return_value(written as u32);
            }
            _ => {
                crate::pr_warn!("Unsupported fd type for write syscall\n");
                (*regs).set_return_value(!0u32);
            }
        }
    }
}
