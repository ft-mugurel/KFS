use crate::error::{KResult, KernelError};
use crate::locks::Spinlock;
use crate::waitqueue::WaitQueue;

const PIPE_BUF_SIZE: usize = 4096;
const MAX_PIPES: usize = 32;

struct PipeBuffer {
    data: [u8; PIPE_BUF_SIZE],
    read_pos: usize,
    write_pos: usize,
    count: usize, // bytes currently in buffer
    readers: u32,
    writers: u32,
    read_wq: WaitQueue,
    write_wq: WaitQueue,
    in_use: bool,
}

impl PipeBuffer {
    const fn new() -> Self {
        Self {
            data: [0; PIPE_BUF_SIZE],
            read_pos: 0,
            write_pos: 0,
            count: 0,
            readers: 0,
            writers: 0,
            read_wq: WaitQueue::new(),
            write_wq: WaitQueue::new(),
            in_use: false,
        }
    }
}

static PIPES: Spinlock<[PipeBuffer; MAX_PIPES]> =
    Spinlock::new([const { PipeBuffer::new() }; MAX_PIPES]);

pub(crate) type PipeId = usize;

pub(crate) fn create_pipe() -> KResult<PipeId> {
    let mut pipes = PIPES.lock();
    for i in 0..MAX_PIPES {
        if !pipes[i].in_use {
            pipes[i] = PipeBuffer::new();
            pipes[i].in_use = true;
            pipes[i].readers = 1;
            pipes[i].writers = 1;
            return Ok(i);
        }
    }
    Err(KernelError::ENFILE)
}

pub(crate) unsafe fn pipe_read(pipe_id: PipeId, buf: &mut [u8]) -> KResult<usize> {
    if pipe_id >= MAX_PIPES {
        return Err(KernelError::EBADF);
    }

    let mut pipes = PIPES.lock();
    let pipe = &mut pipes[pipe_id];

    if !pipe.in_use {
        return Err(KernelError::EBADF);
    }

    if pipe.count == 0 {
        if pipe.writers == 0 {
            return Ok(0);
        }
        pipe.read_wq.sleep();
        drop(pipes);
        return Err(KernelError::EAGAIN);
    }

    let to_read = buf.len().min(pipe.count);
    for i in 0..to_read {
        buf[i] = pipe.data[pipe.read_pos];
        pipe.read_pos = (pipe.read_pos + 1) % PIPE_BUF_SIZE;
    }
    pipe.count -= to_read;

    pipe.write_wq.wake_one();

    Ok(to_read)
}

pub(crate) unsafe fn pipe_write(pipe_id: PipeId, buf: &[u8]) -> KResult<usize> {
    if pipe_id >= MAX_PIPES {
        return Err(KernelError::EBADF);
    }

    let mut pipes = PIPES.lock();
    let pipe = &mut pipes[pipe_id];

    if !pipe.in_use {
        return Err(KernelError::EBADF);
    }

    if pipe.readers == 0 {
        return Err(KernelError::EPIPE);
    }

    if pipe.count >= PIPE_BUF_SIZE {
        pipe.write_wq.sleep();
        drop(pipes);
        return Err(KernelError::EAGAIN);
    }

    let space = PIPE_BUF_SIZE - pipe.count;
    let to_write = buf.len().min(space);
    for i in 0..to_write {
        pipe.data[pipe.write_pos] = buf[i];
        pipe.write_pos = (pipe.write_pos + 1) % PIPE_BUF_SIZE;
    }
    pipe.count += to_write;
    pipe.read_wq.wake_one();

    Ok(to_write)
}

pub(crate) fn close_read_end(pipe_id: PipeId) {
    let mut pipes = PIPES.lock();
    if pipe_id < MAX_PIPES && pipes[pipe_id].in_use {
        pipes[pipe_id].readers = pipes[pipe_id].readers.saturating_sub(1);
        unsafe {
            pipes[pipe_id].write_wq.wake_all();
        }
        if pipes[pipe_id].readers == 0 && pipes[pipe_id].writers == 0 {
            pipes[pipe_id].in_use = false;
        }
    }
}

pub(crate) fn close_write_end(pipe_id: PipeId) {
    let mut pipes = PIPES.lock();
    if pipe_id < MAX_PIPES && pipes[pipe_id].in_use {
        pipes[pipe_id].writers = pipes[pipe_id].writers.saturating_sub(1);
        unsafe {
            pipes[pipe_id].read_wq.wake_all();
        }
        if pipes[pipe_id].readers == 0 && pipes[pipe_id].writers == 0 {
            pipes[pipe_id].in_use = false;
        }
    }
}
