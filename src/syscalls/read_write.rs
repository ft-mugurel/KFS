use crate::fs::FileDescriptor;
use crate::ipc::{SOCKETS, SOCKET_BUFFER_SIZE};
use crate::sched::{ContextFrame, CURRENT_PID, MAX_FDS_PER_PROCESS, PROCESS_TABLE};
use crate::vga::text_mod::{print_str_on, print_fmt_on};
use crate::{pr_debug, pr_warn};

pub(super) unsafe fn syscall_read(regs: *mut ContextFrame) {
    unsafe {
        let fd = (*regs).arg1() as usize;
        let buf_ptr = (*regs).arg2() as *mut u8;
        let count = (*regs).arg3() as usize;
        let current_pid = CURRENT_PID;

        if fd >= MAX_FDS_PER_PROCESS {
            (*regs).set_return_value(!0u32);
            return;
        }

        let task = PROCESS_TABLE[current_pid].as_mut().unwrap();
        match task.fd_tbl[fd] {
            Some(FileDescriptor::TTY(_)) => {
                pr_warn!("Read syscall on TTY is not yet implemented\n");
                (*regs).set_return_value(!0u32);
            }
            Some(FileDescriptor::Socket(sock_idx)) => {
                let mut sock = SOCKETS[sock_idx].lock();

                let mut read_bytes = 0;
                let slice = core::slice::from_raw_parts_mut(buf_ptr, count);

                while read_bytes < count && sock.head != sock.tail {
                    slice[read_bytes] = sock.buffer[sock.tail];
                    sock.tail = (sock.tail + 1) % SOCKET_BUFFER_SIZE;
                    read_bytes += 1;
                }
                (*regs).set_return_value(read_bytes as u32);
            }
            _ => {
                pr_warn!("Unsupported fd type for read syscall\n");
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

        if fd_index >= MAX_FDS_PER_PROCESS {
            (*regs).set_return_value(!0u32);
            return;
        }

        let task = PROCESS_TABLE[current_pid].as_mut().unwrap();
        let slice = core::slice::from_raw_parts(buf_ptr, count);

        match task.fd_tbl[fd_index] {
            Some(FileDescriptor::TTY(idx)) => {
                if let Ok(s) = core::str::from_utf8(slice) {
                    print_fmt_on(idx, &format_args!("({})", current_pid));
                    print_str_on(idx, s);
                }
                (*regs).set_return_value(count as u32);
            }
            Some(FileDescriptor::Socket(sock_idx)) => {
                pr_debug!("Writing {} bytes to Socket {}\n", count, sock_idx);
                let mut sock = SOCKETS[sock_idx].lock();
                let mut written = 0;
                for &byte in slice {
                    let next_head = (sock.head + 1) % SOCKET_BUFFER_SIZE;
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
                pr_warn!("Unsupported fd type for write syscall\n");
                (*regs).set_return_value(!0u32);
            }
        }
    }
}
