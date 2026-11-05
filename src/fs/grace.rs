//! Delayed freeing of VFS nodes.
//!
//! Path resolution hands out raw node pointers without taking a reference, so
//! a node can be unlinked on one CPU while another is still walking past it or
//! is about to use it. Code using such pointers holds a `NodePin`, and a node
//! is only freed once every pin that could have reached it has been dropped.
//!
//! Pins are counted in two buckets. New pins and newly queued nodes use the
//! current bucket. Once the other bucket is empty the two swap, and the queued
//! nodes wait for their bucket to drain. Pins that keep arriving land in the
//! new bucket, so they can't delay a free forever.
//!
//! Waiting isn't sufficient on its own: a pinned `open` can store a node in the
//! open file table after it was queued. So each node is checked again when its
//! grace period ends. An open file hands it over to the last close, and a
//! directory still in use waits for another round.
//!
//! A pin's drop can take OPEN_FILE_TABLE and PROCESS_TABLE, so it must not
//! happen under any spinlock.

use core::ptr::null_mut;

use super::{OPEN_FILE_TABLE, VfsNode, VfsNodeType, claim_free, free_vfs_node, node_is_open};
use crate::{locks::Spinlock, pr_warn, sched};

struct Grace {
    active: [u32; 2],
    current: usize,
    pending: [*mut VfsNode; 2],
}

unsafe impl Send for Grace {}

impl Grace {
    unsafe fn push(&mut self, node: *mut VfsNode) {
        unsafe { (*node).next_free = self.pending[self.current] };
        self.pending[self.current] = node;
    }

    /// Takes the nodes that no pin can reach any more.
    fn advance(&mut self) -> [*mut VfsNode; 2] {
        let mut ready = [null_mut(); 2];
        for list in &mut ready {
            let older = 1 - self.current;
            if self.active[older] != 0 {
                break;
            }
            *list = core::mem::take(&mut self.pending[older]);
            if self.pending[self.current].is_null() {
                break;
            }
            self.current = older;
        }
        ready
    }
}

static GRACE: Spinlock<Grace> =
    Spinlock::new(Grace { active: [0; 2], current: 0, pending: [null_mut(); 2] });

pub(crate) struct NodePin {
    slot: usize,
}

pub(crate) fn pin_nodes() -> NodePin {
    let mut grace = GRACE.lock();
    let slot = grace.current;
    grace.active[slot] += 1;
    // SAFETY: the task is only touched by itself here, and interrupts are off.
    if let Some(task) = unsafe { sched::current().as_mut() } {
        task.vfs_pins[slot] += 1;
    }
    NodePin { slot }
}

impl Drop for NodePin {
    fn drop(&mut self) {
        let mut grace = GRACE.lock();
        grace.active[self.slot] -= 1;
        // SAFETY: as in `pin_nodes`.
        if let Some(task) = unsafe { sched::current().as_mut() } {
            task.vfs_pins[self.slot] = task.vfs_pins[self.slot].saturating_sub(1);
        }
        let ready = grace.advance();
        drop(grace);
        unsafe { finish(ready) };
    }
}

/// Drops the pins of a task that will never run again.
///
/// # Safety
/// Must not be called under any spinlock.
pub(crate) unsafe fn release_task_pins(pins: [u32; 2]) {
    if pins == [0; 2] {
        return;
    }
    let mut grace = GRACE.lock();
    grace.active[0] -= pins[0];
    grace.active[1] -= pins[1];
    let ready = grace.advance();
    drop(grace);
    unsafe { finish(ready) };
}

/// Queues a node already claimed with `claim_free`.
///
/// # Safety
/// Must not be called under any spinlock, and `node` must be out of the tree
pub(super) unsafe fn queue_free(node: *mut VfsNode) {
    let mut grace = GRACE.lock();
    unsafe { grace.push(node) };
    let ready = grace.advance();
    drop(grace);
    unsafe { finish(ready) };
}

/// # Safety
/// Must not be called under any spinlock, and `node` must be out of the tree
pub(crate) unsafe fn defer_free(node: *mut VfsNode) {
    let claimed = {
        let _table = OPEN_FILE_TABLE.lock();
        unsafe { claim_free(node) }
    };
    if claimed {
        unsafe { queue_free(node) };
    }
}

unsafe fn finish(ready: [*mut VfsNode; 2]) {
    for mut node in ready {
        while !node.is_null() {
            let next = unsafe { (*node).next_free };
            unsafe {
                (*node).next_free = null_mut();
                finish_node(node);
            }
            node = next;
        }
    }
}

unsafe fn finish_node(node: *mut VfsNode) {
    let is_dir = unsafe { (*node).node_type } == VfsNodeType::Directory;
    let open = {
        let table = OPEN_FILE_TABLE.lock();
        let open = node_is_open(&*table, node);
        if open && !is_dir {
            // Under the lock, so the last close sees it unclaimed and frees it.
            unsafe { (*node).free_pending = false };
        }
        open
    };
    if open && !is_dir {
        return;
    }

    let in_use = is_dir
        && (open
            || sched::PROCESS_TABLE
                .lock()
                .iter()
                .flatten()
                .any(|task| task.cwd == node));
    if in_use {
        // Retried on the next pin or free; `finish` must not loop on it here.
        unsafe { GRACE.lock().push(node) };
        return;
    }
    unsafe {
        evict(node);
        free_vfs_node(node);
    }
}

/// Nothing can reach the node now, so an unlinked inode's data can go too.
unsafe fn evict(node: *mut VfsNode) {
    let (links, master, inode) = unsafe { ((*node).links, (*node).master, (*node).inode) };
    if links != 0 || master.is_null() {
        return;
    }
    let mount = unsafe { (*master).mount };
    if mount.is_null() {
        return;
    }
    if let Err(e) = unsafe { ((*mount).backend.evict)(mount, inode) } {
        pr_warn!("VFS: failed to free inode {}: {:?}", inode, e);
    }
}
