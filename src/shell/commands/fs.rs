use core::str;
use crate::drivers;
use crate::fs;
use crate::sched;
use crate::shell::init::{print, print_fmt};
use super::parse::parse_usize;

#[inline(always)]
pub(crate) fn command_devices() {
    print("ID  NAME       KIND       PARENT  START     SECTORS\n");
    for device_id in 0..drivers::MAX_BLOCK_DEVICES {
        let Some(device) = drivers::device_at(device_id) else {
            continue;
        };
        let name_len = device
            .name
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(device.name.len());
        let name = core::str::from_utf8(&device.name[..name_len]).unwrap_or("?");
        let kind = if device.partition {
            "partition"
        } else {
            "disk"
        };
        print_fmt(&format_args!(
            "{:<3} {:<10} {:<10} {:<7} {:<9} {}\n",
            device_id, name, kind, device.parent, device.start_lba, device.sector_count
        ));
    }
}

#[inline(always)]
pub(crate) fn command_storage_test(mut parts: str::SplitWhitespace<'_>) {
    let Some(device_str) = parts.next() else {
        print("usage: storage_test <device-id>\n");
        return;
    };
    let Some(device_id) = parse_usize(device_str) else {
        print("invalid device id\n");
        return;
    };
    let Some(device) = drivers::device_at(device_id) else {
        print("device not found\n");
        return;
    };

    let mut buffer = [0u8; 1024];
    if drivers::read_sectors(device_id, 0, 1, &mut buffer).is_err() {
        print("device read failed\n");
        return;
    }

    if device.partition {
        if drivers::read_sectors(device_id, 2, 2, &mut buffer).is_err() {
            print("partition superblock read failed\n");
            return;
        }
        let magic = u16::from_le_bytes([buffer[56], buffer[57]]);
        print_fmt(&format_args!(
            "device {} EXT2 magic: {:#06x}\n",
            device_id, magic
        ));
        if magic == 0xEF53 {
            print("EXT2 superblock valid\n");
        } else {
            print("invalid EXT2 superblock\n");
        }
    } else if buffer[510] == 0x55 && buffer[511] == 0xAA {
        print("valid MBR signature\n");
        for index in 0..4 {
            let offset = 446 + index * 16;
            let partition_type = buffer[offset + 4];
            let start = u32::from_le_bytes([
                buffer[offset + 8],
                buffer[offset + 9],
                buffer[offset + 10],
                buffer[offset + 11],
            ]);
            let sectors = u32::from_le_bytes([
                buffer[offset + 12],
                buffer[offset + 13],
                buffer[offset + 14],
                buffer[offset + 15],
            ]);
            if partition_type != 0 && sectors != 0 {
                print_fmt(&format_args!(
                    "partition {}: type {:#04x}, start {}, sectors {}\n",
                    index + 1,
                    partition_type,
                    start,
                    sectors
                ));
            }
        }
    } else {
        print("no valid MBR signature\n");
    }
}

#[inline(always)]
pub(crate) unsafe fn command_mount(mut parts: str::SplitWhitespace<'_>) {
    let Some(device_str) = parts.next() else {
        print("usage: mount <device-id> <target>\n");
        return;
    };
    let Some(device_id) = parse_usize(device_str) else {
        print("invalid device id\n");
        return;
    };
    let Some(target_path) = parts.next() else {
        print("usage: mount <device-id> <target>\n");
        return;
    };
    let cwd = (*sched::current().as_mut().unwrap()).cwd;
    let target = match fs::resolve_path(target_path, cwd) {
        Ok(node) => node,
        Err(error) => {
            print_fmt(&format_args!("mount target lookup failed: {:?}\n", error));
            return;
        }
    };
    match fs::ext2::mount_device_at(device_id, target) {
        Ok(()) => print("filesystem mounted\n"),
        Err(error) => print_fmt(&format_args!("mount failed: {:?}\n", error)),
    }
}

#[inline(always)]
pub(crate) unsafe fn command_mkdir(mut parts: str::SplitWhitespace<'_>) {
    let Some(path) = parts.next() else {
        print("usage: mkdir <path>\n");
        return;
    };
    if path.is_empty() || path == "/" {
        print("invalid directory path\n");
        return;
    }

    let (parent_path, name) = match path.rsplit_once('/') {
        Some((parent, name)) => (if parent.is_empty() { "/" } else { parent }, name),
        None => ("", path),
    };
    if name.is_empty() {
        print("invalid directory path\n");
        return;
    }

    let cwd = (*sched::current().as_mut().unwrap()).cwd;
    let parent = match fs::resolve_path(parent_path, cwd) {
        Ok(node) => node,
        Err(error) => {
            print_fmt(&format_args!("mkdir parent lookup failed: {:?}\n", error));
            return;
        }
    };
    if (*parent).node_type != fs::VfsNodeType::Directory {
        print("mkdir parent is not a directory\n");
        return;
    }

    match fs::create_child_node(parent, name, fs::VfsNodeType::Directory, 0o755) {
        Ok(_) => print("directory created\n"),
        Err(error) => print_fmt(&format_args!("mkdir failed: {:?}\n", error)),
    }
}

#[inline(always)]
pub(crate) unsafe fn command_umount(mut parts: str::SplitWhitespace<'_>) {
    let Some(target_path) = parts.next() else {
        print("usage: umount <target>\n");
        return;
    };
    let cwd = (*sched::current().as_mut().unwrap()).cwd;
    let target = match fs::resolve_path(target_path, cwd) {
        Ok(node) => node,
        Err(error) => {
            print_fmt(&format_args!("unmount target lookup failed: {:?}\n", error));
            return;
        }
    };
    if target == fs::ROOT_NODE {
        print("cannot unmount the root filesystem\n");
        return;
    }
    (*target).children = core::ptr::null_mut();
    match fs::umount_node(target) {
        Ok(()) => print("filesystem unmounted\n"),
        Err(error) => print_fmt(&format_args!("unmount failed: {:?}\n", error)),
    }
}

#[inline(always)]
pub(crate) fn command_fs_test() {
    unsafe {
        let dev_path: [u8; 5] = [b'/', b'd', b'e', b'v', 0];
        let mnt_path: [u8; 5] = [b'/', b'm', b'n', b't', 0];
        let file_path: [u8; 13] = [
            b'/', b'f', b's', b'_', b't', b'e', b's', b't', b'.', b't', b'x', b't', 0,
        ];
        let write_msg: [u8; 6] = [b'f', b's', b'-', b'o', b'k', b'\n'];
        let mut result: u32;
        let mut fd: u32;
        let mut read_back: u32;
        let mut read_buf = [0u8; 32];

        core::arch::asm!(
            "int 0x80",
            in("eax") 14,
            in("ebx") mnt_path.as_ptr(),
            in("ecx") 0x4000u32 | 0o755,
            lateout("eax") _,
            options(nostack, nomem),
        );

        core::arch::asm!(
            "int 0x80",
            in("eax") 21,
            in("ebx") dev_path.as_ptr(),
            in("ecx") mnt_path.as_ptr(),
            lateout("eax") result,
            options(nostack, nomem),
        );
        if (result as i32) < 0 {
            print("fs_test: mount failed\n");
            return;
        }

        core::arch::asm!(
            "int 0x80",
            in("eax") 52,
            in("ebx") mnt_path.as_ptr(),
            lateout("eax") result,
            options(nostack, nomem),
        );
        if (result as i32) < 0 {
            print("fs_test: umount failed\n");
            return;
        }

        core::arch::asm!(
            "int 0x80",
            in("eax") 5,
            in("ebx") file_path.as_ptr(),
            in("ecx") 0x241u32, // O_CREAT | O_TRUNC | O_RDWR
            in("edx") 0o644u32,
            lateout("eax") fd,
            options(nostack, nomem),
        );
        if (fd as i32) < 0 {
            print("fs_test: open create/trunc failed\n");
            return;
        }

        core::arch::asm!(
            "int 0x80",
            in("eax") 4,
            in("ebx") fd,
            in("ecx") write_msg.as_ptr(),
            in("edx") write_msg.len(),
            lateout("eax") _,
            options(nostack, nomem),
        );
        core::arch::asm!("int 0x80", in("eax") 6, in("ebx") fd, options(nostack, nomem));

        core::arch::asm!(
            "int 0x80",
            in("eax") 5,
            in("ebx") file_path.as_ptr(),
            in("ecx") 0u32,
            in("edx") 0u32,
            lateout("eax") fd,
            options(nostack, nomem),
        );
        if (fd as i32) < 0 {
            print("fs_test: reopen failed\n");
            return;
        }

        core::arch::asm!(
            "int 0x80",
            in("eax") 3,
            in("ebx") fd,
            in("ecx") read_buf.as_mut_ptr(),
            in("edx") read_buf.len(),
            lateout("eax") read_back,
            options(nostack, nomem),
        );
        core::arch::asm!("int 0x80", in("eax") 6, in("ebx") fd, options(nostack, nomem));

        if (read_back as i32) < 0 {
            print("fs_test: readback failed\n");
            return;
        }

        print("fs_test: success\n");
        if let Ok(text) = core::str::from_utf8(&read_buf[..read_back as usize]) {
            print(text);
            print("\n");
        }
    }
}

#[inline(always)]
pub(crate) fn command_fs_tree() {
    unsafe {
        fs::print_vfs_tree(fs::ROOT_NODE, 0);
    }
}
