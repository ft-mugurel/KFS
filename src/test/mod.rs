use crate::fs::{self, VfsNodeType};
use crate::pr_info;

pub(crate) fn fs_boot_probe() {
    unsafe {
        let root = fs::ROOT_NODE;
        if root.is_null() {
            pr_info!("fs_boot_probe: root not ready\n");
            return;
        }

        let dev_dir = match fs::resolve_path("/dev", root) {
            Ok(node) => node,
            Err(err) => {
                pr_info!("fs_boot_probe: /dev unavailable: {:?}\n", err);
                return;
            }
        };

        let mnt_dir = match fs::resolve_path("/mnt", root) {
            Ok(node) => node,
            Err(_) => match fs::create_child_node(root, "mnt", VfsNodeType::Directory, 0o755) {
                Ok(node) => node,
                Err(err) => {
                    pr_info!("fs_boot_probe: could not create /mnt: {:?}\n", err);
                    return;
                }
            },
        };

        if let Err(err) = fs::mount_node(mnt_dir, dev_dir) {
            pr_info!("fs_boot_probe: mount helper failed: {:?}\n", err);
            return;
        }

        if let Err(err) = fs::umount_node(mnt_dir) {
            pr_info!("fs_boot_probe: umount helper failed: {:?}\n", err);
            return;
        }

        pr_info!("fs_boot_probe: mount helpers validated\n");
    }
}

#[unsafe(no_mangle)]
pub(crate) unsafe fn process_socket() {
    let mut fd: u32;
    core::arch::asm!(
        "int 0x80",
        in("eax") 97,
        in("ebx") 1, // AF_UNIX
        in("ecx") 1, // SOCK_STREAM
        in("edx") 0, // protocol
        lateout("eax") fd,
        options(nostack, nomem)
    );

    let out_msg: [u8; 5] = [b'I', b'P', b'C', b'!', b'\n'];
    core::arch::asm!(
        "int 0x80",
        in("eax") 4,
        in("ebx") fd,
        in("ecx") out_msg.as_ptr(),
        in("edx") out_msg.len(),
        options(nostack),
    );

    let mut in_buffer: [u8; 8] = [0; 8];
    let mut read_size: u32;
    core::arch::asm!(
        "int 0x80",
        in("eax") 3,
        in("ebx") fd,
        in("ecx") in_buffer.as_mut_ptr(),
        in("edx") in_buffer.len(),
        lateout("eax") read_size,
        options(nostack),
    );

    core::arch::asm!(
        "int 0x80",
        in("eax") 4,
        in("ebx") 1,
        in("ecx") in_buffer.as_ptr(),
        in("edx") read_size,
        options(nostack),
    );

    core::arch::asm!("int 0x80", in("eax") 1, in("ebx") 0, options(noreturn));
}

/// Current flow:
///   1. Parent process creates a socket and forks a child process.
///   2. Child process opens the file "/test.txt" and reads 32 bytes from it,
///      then sends the data to the parent process through the socket.
///   3. Parent process waits for the child process to exit,
///      then reads the data from the socket and prints it to stdout.
///
/// Or in pseudo-code:
/// ```
///  sock_fd = socket(AF_UNIX, SOCK_STREAM, 0)
///  pid = fork()
///  if pid == 0 {
///      ext2_fd = open("/test.txt", O_RDONLY)
///      read_size = read(ext2_fd, ext2_fd_read_buf, 32)
///      write(sock_fd, ext2_fd_read_buf, ext2_fd_read_size)
///      exit(0)
///  } else if pid > 0 {
///      write(1, "Waiting\n", 8)
///      wait(NULL)
///      write(1, "Waked\n", 6)
///      read_size = read(sock_fd, parent_buf, 32)
///      write(1, parent_buf, read_size)
///      exit(0)
///  } else {
///      exit(1)
///  }
/// ```
///
#[unsafe(no_mangle)]
pub(crate) unsafe fn most_syscalls_we_have_probably() {
    let sock_fd: u32;
    let pid: u32;
    let slp_msg = [b'W', b'a', b'i', b't', b'i', b'n', b'g', b'\n'];
    let wake_msg = [b'W', b'a', b'k', b'e', b'd', b'\n'];
    let read_size: u32;
    let mut parent_buf = [0u8; 32];

    // sock_fd = socket(AF_UNIX, SOCK_STREAM, 0)
    core::arch::asm!(
        "int 0x80",
        in("eax") 97,
        in("ebx") 1, // AF_UNIX
        in("ecx") 1, // SOCK_STREAM
        in("edx") 0, // protocol
        lateout("eax") sock_fd,
        options(nostack, nomem)
    );
    // pid = fork()
    core::arch::asm!(
        "int 0x80",
        in("eax") 2,
        lateout("eax") pid,
        options(nostack)
    );
    if pid == 0 {
        let ext2_fd: u32;
        let file_name: [u8; 10] = [b'/', b't', b'e', b's', b't', b'.', b't', b'x', b't', 0];
        core::arch::asm!(
            "int 0x80",
            in("eax") 5,
            in("ebx") file_name.as_ptr(),
            in("ecx") 0,
            in("edx") 0,
            lateout("eax") ext2_fd,
            options(nostack, nomem)
        );
        let mut ext2_fd_read_buf: [u8; 32] = [0; 32];
        let mut ext2_fd_read_size: u32;
        core::arch::asm!(
            "int 0x80",
            in("eax") 3,
            in("ebx") ext2_fd,
            in("ecx") ext2_fd_read_buf.as_mut_ptr(),
            in("edx") ext2_fd_read_buf.len(),
            lateout("eax") ext2_fd_read_size,
            options(nostack)
        );
        core::arch::asm!(
            "int 0x80",
            in("eax") 4,
            in("ebx") sock_fd,
            in("ecx") ext2_fd_read_buf.as_ptr(),
            in("edx") ext2_fd_read_size,
            options(nostack)
        );
        core::arch::asm!("int 0x80", in("eax") 1, in("ebx") 0, options(noreturn));
    } else if pid > 0 {
        core::arch::asm!(
            "int 0x80",
            in("eax") 4,
            in("ebx") 1,
            in("ecx") slp_msg.as_ptr(),
            in("edx") slp_msg.len(),
            options(nostack)
        );
        core::arch::asm!("int 0x80", in("eax") 7, options(nostack));
        core::arch::asm!(
            "int 0x80",
            in("eax") 4,
            in("ebx") 1,
            in("ecx") wake_msg.as_ptr(),
            in("edx") wake_msg.len(),
            options(nostack)
        );
        core::arch::asm!(
            "int 0x80",
            in("eax") 3,
            in("ebx") sock_fd,
            in("ecx") parent_buf.as_mut_ptr(),
            in("edx") parent_buf.len(),
            lateout("eax") read_size,
            options(nostack)
        );
        core::arch::asm!(
            "int 0x80",
            in("eax") 4,
            in("ebx") 1,
            in("ecx") parent_buf.as_ptr(),
            in("edx") read_size,
            options(nostack),
        );
        core::arch::asm!("int 0x80", in("eax") 1, in("ebx") 0, options(noreturn));
    } else {
        core::arch::asm!("int 0x80", in("eax") 1, in("ebx") 1, options(noreturn));
    }
}
