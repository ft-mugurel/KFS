//! Snapshot reads for generated pseudo-files (/proc, /sys)
//!
//! The content is generated whole when a reader starts at offset 0 and kept per
//! (reader PID, mount, inode), so a file read in several chunks stays
//! consistent. The buffer grows until the generator stops filling it.

use crate::{error::KResult, locks::Spinlock, paging, sched};

use super::Mount;

const INITIAL_CAPACITY: usize = 4096;
const MAX_CAPACITY: usize = 64 * 1024;
const SLOTS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    pid: u32,
    mount: usize,
    inode: u32,
}

#[derive(Clone, Copy)]
struct Snapshot {
    key: Key,
    data: *mut u8,
    len: usize,
}

struct Cache {
    slots: [Option<Snapshot>; SLOTS],
    next_victim: usize,
}

unsafe impl Send for Cache {}

static CACHE: Spinlock<Cache> = Spinlock::new(Cache { slots: [None; SLOTS], next_victim: 0 });

fn copy_at(data: &[u8], offset: usize, buffer: &mut [u8]) -> usize {
    if offset >= data.len() {
        return 0;
    }
    let count = (data.len() - offset).min(buffer.len());
    buffer[..count].copy_from_slice(&data[offset..offset + count]);
    count
}

/// Runs `generate` into a buffer large enough to hold its whole output.
/// Output that fills the buffer is assumed truncated and regenerated larger.
fn generate_full(
    generate: &mut dyn FnMut(&mut [u8]) -> KResult<usize>,
) -> KResult<(*mut u8, usize)> {
    let mut capacity = INITIAL_CAPACITY;
    loop {
        let data = paging::kmalloc(capacity)?;
        let len = match generate(unsafe { core::slice::from_raw_parts_mut(data, capacity) }) {
            Ok(len) => len,
            Err(err) => {
                let _ = paging::kfree(data);
                return Err(err);
            }
        };
        if len < capacity || capacity >= MAX_CAPACITY {
            return Ok((data, len.min(capacity)));
        }
        let _ = paging::kfree(data);
        capacity *= 2;
    }
}

pub unsafe fn read(
    mount: *const Mount,
    inode: u32,
    buffer: &mut [u8],
    offset: u32,
    generate: &mut dyn FnMut(&mut [u8]) -> KResult<usize>,
) -> KResult<usize> {
    let key = Key { pid: sched::current_pid(), mount: mount as usize, inode };
    let offset = offset as usize;

    if offset > 0 {
        let cache = CACHE.lock();
        if let Some(snapshot) = cache.slots.iter().flatten().find(|s| s.key == key) {
            let data = core::slice::from_raw_parts(snapshot.data, snapshot.len);
            return Ok(copy_at(data, offset, buffer));
        }
    }

    // Generate without holding CACHE: generators take other locks.
    let (data, len) = generate_full(generate)?;
    let copied = copy_at(core::slice::from_raw_parts(data, len), offset, buffer);

    let evicted = {
        let mut cache = CACHE.lock();
        let index = match cache
            .slots
            .iter()
            .position(|s| s.is_some_and(|s| s.key == key))
        {
            Some(index) => index,
            None => match cache.slots.iter().position(Option::is_none) {
                Some(index) => index,
                None => {
                    let index = cache.next_victim;
                    cache.next_victim = (index + 1) % SLOTS;
                    index
                }
            },
        };
        cache.slots[index].replace(Snapshot { key, data, len })
    };
    if let Some(old) = evicted {
        let _ = paging::kfree(old.data);
    }
    Ok(copied)
}
