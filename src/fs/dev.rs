use crate::{
    drivers,
    error::{KResult, KernelError},
    fs::{self, VfsNode, VfsNodeType},
    interrupts::timer::read_tsc,
    locks::Spinlock,
    sched,
    vga::text_mod::active_screen_index,
};

// pub const CHAR_DEV_TTY1: u32 = 0;
// pub const CHAR_DEV_TTY2: u32 = 1;
// pub const CHAR_DEV_TTY3: u32 = 2;
// pub const CHAR_DEV_TTY4: u32 = 3;
// pub const CHAR_DEV_TTY5: u32 = 4;
// pub const CHAR_DEV_TTY6: u32 = 5;
pub const CHAR_DEV_TTY0: u32 = 6;
pub const CHAR_DEV_CONSOLE: u32 = 7;
pub const CHAR_DEV_TTY: u32 = 8;
pub const CHAR_DEV_NULL: u32 = 9;
pub const CHAR_DEV_ZERO: u32 = 10;
pub const CHAR_DEV_URANDOM: u32 = 11;
pub const CHAR_DEV_RANDOM: u32 = 12;

pub const CHAR_DRIVERS: &[(u32, &str)] =
    &[(1, "mem"), (4, "tty"), (5, "/dev/tty"), (5, "/dev/console")];

pub const IDE_MAJORS: [u32; 4] = [3, 22, 33, 34];

const TTY_COUNT: usize = crate::startup_config::vga::VIRTUAL_SCREENS;

static PRNG_STATE: Spinlock<u64> = Spinlock::new(0x853c49e6748fea9b);

pub fn get_random_bytes(buf: &mut [u8]) {
    let mut state = PRNG_STATE.lock();
    if *state == 0 {
        *state = read_tsc() ^ 0xdeadbeef_cafebabe;
    }
    for chunk in buf.chunks_mut(8) {
        let mut x = *state ^ read_tsc();
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
    if let Some(task) = unsafe { sched::current().as_ref() }
        && let Some(gfd) = task.fd_tbl[0]
        && let Some(f) = fs::get_open_file(gfd)
    {
        unsafe {
            if (*f.node).node_type == VfsNodeType::CharDevice
                && ((*f.node).inode as usize) < TTY_COUNT
            {
                return (*f.node).inode as usize;
            }
        }
    }
    active_screen_index()
}

pub fn read_char_device(inode: u32, buffer: &mut [u8]) -> KResult<usize> {
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
        _ => Err(KernelError::NoDev),
    }
}

pub fn write_char_device(inode: u32, buffer: &[u8]) -> KResult<usize> {
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
        _ => Err(KernelError::NoDev),
    }
}

#[unsafe(link_section = ".init.text")]
pub unsafe fn init() -> KResult<()> {
    unsafe fn add(
        dev: *mut VfsNode,
        name: &str,
        node_type: VfsNodeType,
        mode: u16,
        inode: u32,
        size: u32,
    ) -> KResult<()> {
        let node = fs::create_child_node(dev, name, node_type, mode)?;
        (*node).inode = inode;
        (*node).size = size;
        Ok(())
    }

    let dev = fs::resolve_path("/dev", fs::ROOT_NODE)?;

    // 1. Virtual screens /dev/tty1..6
    for index in 0..TTY_COUNT {
        let (name, name_len) = crate::tty::tty_node_name(index);
        let name = core::str::from_utf8(&name[..name_len]).unwrap();
        add(dev, name, VfsNodeType::CharDevice, 0o666, index as u32, 0)?;
    }

    // 2. Virtual console aliases /dev/tty0, /dev/console, /dev/tty
    // 3. Standard pseudo-devices /dev/null, /dev/zero, /dev/urandom, /dev/random
    for (name, inode) in [
        ("tty0", CHAR_DEV_TTY0),
        ("console", CHAR_DEV_CONSOLE),
        ("tty", CHAR_DEV_TTY),
        ("null", CHAR_DEV_NULL),
        ("zero", CHAR_DEV_ZERO),
        ("urandom", CHAR_DEV_URANDOM),
        ("random", CHAR_DEV_RANDOM),
    ] {
        add(dev, name, VfsNodeType::CharDevice, 0o666, inode, 0)?;
    }

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

            let size = blk.sector_count * blk.sector_size as u32;
            add(
                dev,
                name_str,
                VfsNodeType::BlockDevice,
                0o660,
                dev_id as u32,
                size,
            )?;
        }
    }

    fs::mark_dev_ready();
    crate::pr_info!("dev: all device nodes populated in /dev");
    Ok(())
}
