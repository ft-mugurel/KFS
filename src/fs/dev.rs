use crate::{
    drivers,
    error::{KResult, KernelError},
    fs::{self, VfsNodeType},
    locks::Spinlock,
    sched,
    vga::text_mod::active_screen_index,
};

pub const CHAR_DEV_TTY1: u32 = 0;
pub const CHAR_DEV_TTY2: u32 = 1;
pub const CHAR_DEV_TTY3: u32 = 2;
pub const CHAR_DEV_TTY4: u32 = 3;
pub const CHAR_DEV_TTY5: u32 = 4;
pub const CHAR_DEV_TTY6: u32 = 5;
pub const CHAR_DEV_TTY0: u32 = 6;
pub const CHAR_DEV_CONSOLE: u32 = 7;
pub const CHAR_DEV_TTY: u32 = 8;
pub const CHAR_DEV_NULL: u32 = 9;
pub const CHAR_DEV_ZERO: u32 = 10;
pub const CHAR_DEV_URANDOM: u32 = 11;
pub const CHAR_DEV_RANDOM: u32 = 12;

const TTY_COUNT: usize = crate::startup_config::vga::VIRTUAL_SCREENS;

static PRNG_STATE: Spinlock<u64> = Spinlock::new(0x853c49e6748fea9b);

unsafe fn read_tsc() -> u64 {
    let low: u32;
    let high: u32;
    core::arch::asm!(
        "rdtsc",
        out("eax") low,
        out("edx") high,
        options(nomem, nostack, preserves_flags),
    );
    ((high as u64) << 32) | low as u64
}

pub fn get_random_bytes(buf: &mut [u8]) {
    let mut state = PRNG_STATE.lock();
    if *state == 0 {
        *state = unsafe { read_tsc() ^ 0xdeadbeef_cafebabe };
    }
    for chunk in buf.chunks_mut(8) {
        let mut x = *state ^ (unsafe { read_tsc() });
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        *state = x;
        let random_val = x.wrapping_mul(0x2545F4914F6CDD1D);
        let bytes = random_val.to_ne_bytes();
        chunk.copy_from_slice(&bytes[..chunk.len()]);
    }
}

pub fn get_controlling_tty_index() -> usize {
    if let Some(task) = unsafe { sched::current().as_ref() } {
        if let Some(gfd) = task.fd_tbl[0] {
            if let Some(f) = fs::get_open_file(gfd) {
                unsafe {
                    if (*f.node).node_type == VfsNodeType::CharDevice {
                        let inode = (*f.node).inode;
                        if (inode as usize) < TTY_COUNT {
                            return inode as usize;
                        }
                    }
                }
            }
        }
    }
    active_screen_index()
}

pub unsafe fn read_char_device(inode: u32, buffer: &mut [u8]) -> KResult<usize> {
    match inode {
        0..=5 => crate::tty::read(inode as usize, buffer),
        CHAR_DEV_TTY0 | CHAR_DEV_CONSOLE => {
            let active = active_screen_index();
            crate::tty::read(active, buffer)
        }
        CHAR_DEV_TTY => {
            let tty_idx = get_controlling_tty_index();
            crate::tty::read(tty_idx, buffer)
        }
        CHAR_DEV_NULL => Ok(0), // EOF immediately
        CHAR_DEV_ZERO => {
            buffer.fill(0);
            Ok(buffer.len())
        }
        CHAR_DEV_URANDOM | CHAR_DEV_RANDOM => {
            get_random_bytes(buffer);
            Ok(buffer.len())
        }
        _ => Err(KernelError::ENODEV),
    }
}

pub unsafe fn write_char_device(inode: u32, buffer: &[u8]) -> KResult<usize> {
    match inode {
        0..=5 => crate::tty::write(inode as usize, buffer),
        CHAR_DEV_TTY0 | CHAR_DEV_CONSOLE => {
            let active = active_screen_index();
            crate::tty::write(active, buffer)
        }
        CHAR_DEV_TTY => {
            let tty_idx = get_controlling_tty_index();
            crate::tty::write(tty_idx, buffer)
        }
        CHAR_DEV_NULL | CHAR_DEV_ZERO => Ok(buffer.len()),
        CHAR_DEV_URANDOM | CHAR_DEV_RANDOM => {
            // Mix written bytes into PRNG state
            let mut sum = 0u64;
            for &b in buffer.iter().take(8) {
                sum = (sum << 8) | (b as u64);
            }
            if sum != 0 {
                let mut state = PRNG_STATE.lock();
                *state ^= sum;
            }
            Ok(buffer.len())
        }
        _ => Err(KernelError::ENODEV),
    }
}

#[unsafe(link_section = ".init.text")]
pub unsafe fn init() -> KResult<()> {
    let root = fs::ROOT_NODE;
    let dev = fs::resolve_path("/dev", root)?;

    // 1. Virtual screens /dev/tty1..6
    for index in 0..TTY_COUNT {
        let (name, name_len) = crate::tty::tty_node_name(index);
        let name = core::str::from_utf8(&name[..name_len]).unwrap();
        let node = fs::create_child_node(dev, name, VfsNodeType::CharDevice, 0o666)?;
        (*node).inode = index as u32;
    }

    // 2. Virtual console aliases /dev/tty0, /dev/console, /dev/tty
    let tty0 = fs::create_child_node(dev, "tty0", VfsNodeType::CharDevice, 0o666)?;
    (*tty0).inode = CHAR_DEV_TTY0;

    let console = fs::create_child_node(dev, "console", VfsNodeType::CharDevice, 0o666)?;
    (*console).inode = CHAR_DEV_CONSOLE;

    let tty = fs::create_child_node(dev, "tty", VfsNodeType::CharDevice, 0o666)?;
    (*tty).inode = CHAR_DEV_TTY;

    // 3. Standard pseudo-devices /dev/null, /dev/zero, /dev/urandom, /dev/random
    let null = fs::create_child_node(dev, "null", VfsNodeType::CharDevice, 0o666)?;
    (*null).inode = CHAR_DEV_NULL;

    let zero = fs::create_child_node(dev, "zero", VfsNodeType::CharDevice, 0o666)?;
    (*zero).inode = CHAR_DEV_ZERO;

    let urandom = fs::create_child_node(dev, "urandom", VfsNodeType::CharDevice, 0o666)?;
    (*urandom).inode = CHAR_DEV_URANDOM;

    let random = fs::create_child_node(dev, "random", VfsNodeType::CharDevice, 0o666)?;
    (*random).inode = CHAR_DEV_RANDOM;

    // 4. Block devices from driver registry
    for dev_id in 0..drivers::MAX_BLOCK_DEVICES {
        if let Some(blk) = drivers::device_at(dev_id) {
            let name_len = blk
                .name
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(blk.name.len());
            if name_len == 0 {
                continue;
            }
            let name_str = match core::str::from_utf8(&blk.name[..name_len]) {
                Ok(s) => s.trim_end_matches('\0'),
                Err(_) => continue,
            };
            if name_str.is_empty() {
                continue;
            }

            let node = fs::create_child_node(dev, name_str, VfsNodeType::BlockDevice, 0o660)?;
            (*node).inode = dev_id as u32;
            (*node).size = blk.sector_count * blk.sector_size as u32;

            // Add standard aliases for whole disks: ide0 -> hda, ide1 -> hdb
            if name_str == "ide0" {
                let alias = fs::create_child_node(dev, "hda", VfsNodeType::BlockDevice, 0o660)?;
                (*alias).inode = dev_id as u32;
                (*alias).size = (*node).size;
            } else if name_str == "ide1" {
                let alias = fs::create_child_node(dev, "hdb", VfsNodeType::BlockDevice, 0o660)?;
                (*alias).inode = dev_id as u32;
                (*alias).size = (*node).size;
            }
        }
    }

    fs::mark_dev_ready();
    crate::pr_info!("dev: all device nodes populated in /dev\n");
    Ok(())
}
