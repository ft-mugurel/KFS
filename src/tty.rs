use crate::error::{KResult, KernelError};
use crate::fs::{self, VfsNodeType};
use crate::locks::Spinlock;
use crate::sched::{self, MAX_FDS_PER_PROCESS};
use crate::vga::text_mod::{active_screen_index, print_str_on};

const BUFFER_SIZE: usize = 256;
const TTY_COUNT: usize = crate::startup_config::vga::VIRTUAL_SCREENS;

#[derive(Clone, Copy)]
struct TtyBuffer {
    data: [u8; BUFFER_SIZE],
    head: usize,
    tail: usize,
}

const EMPTY_BUFFER: TtyBuffer = TtyBuffer { data: [0; BUFFER_SIZE], head: 0, tail: 0 };
static TTY_BUFFERS: Spinlock<[TtyBuffer; TTY_COUNT]> = Spinlock::new([EMPTY_BUFFER; TTY_COUNT]);

fn tty_node_name(index: usize) -> ([u8; 8], usize) {
    let mut name = [0; 8];
    name[0] = b't';
    name[1] = b't';
    name[2] = b'y';
    let digit = index + 1;
    if digit >= 10 {
        name[3] = b'0' + (digit / 10) as u8;
        name[4] = b'0' + (digit % 10) as u8;
        (name, 5)
    } else {
        name[3] = b'0' + digit as u8;
        (name, 4)
    }
}

fn tty_index(index: usize) -> KResult<usize> {
    if index < TTY_COUNT {
        Ok(index)
    } else {
        Err(KernelError::ENXIO)
    }
}

pub fn push_char(c: char) {
    let tty = active_screen_index();
    let Ok(tty) = tty_index(tty) else {
        return;
    };

    let mut buffers = TTY_BUFFERS.lock();
    let buffer = &mut buffers[tty];
    let next_head = (buffer.head + 1) % BUFFER_SIZE;
    if next_head != buffer.tail && c.is_ascii() {
        buffer.data[buffer.head] = c as u8;
        buffer.head = next_head;
    }
}

pub unsafe fn read(tty: usize, output: &mut [u8]) -> KResult<usize> {
    let tty = tty_index(tty)?;
    let mut buffers = TTY_BUFFERS.lock();
    let buffer = &mut buffers[tty];
    let mut read = 0;
    while read < output.len() && buffer.tail != buffer.head {
        output[read] = buffer.data[buffer.tail];
        buffer.tail = (buffer.tail + 1) % BUFFER_SIZE;
        read += 1;
    }

    if read == 0 {
        Err(KernelError::EAGAIN)
    } else {
        Ok(read)
    }
}

pub unsafe fn write(tty: usize, input: &[u8]) -> KResult<usize> {
    tty_index(tty)?;
    let text = core::str::from_utf8(input).map_err(|_| KernelError::EILSEQ)?;
    print_str_on(tty, text);
    Ok(input.len())
}

pub unsafe fn init() -> KResult<()> {
    let root = fs::ROOT_NODE;
    let dev = fs::resolve_path("/dev", root)?;

    for index in 0..TTY_COUNT {
        let (name, name_len) = tty_node_name(index);
        let name = core::str::from_utf8(&name[..name_len]).unwrap();
        let node = fs::create_child_node(dev, name, VfsNodeType::CharDevice, 0o620)?;
        (*node).inode = index as u32;
    }

    Ok(())
}

pub(crate) unsafe fn bind_stdio() -> KResult<()> {
    let node = fs::resolve_path("/dev/tty1", fs::ROOT_NODE)?;
    let task = sched::current().as_mut().ok_or(KernelError::ENXIO)?;

    let global_fd = fs::alloc_open_file(node, 3)?;

    for fd in 0..3.min(MAX_FDS_PER_PROCESS) {
        task.fd_tbl[fd] = Some(global_fd);
    }

    Ok(())
}
