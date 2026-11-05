/// # Lock Ordering Discipline (SMP Deadlock Prevention)
///
/// When acquiring multiple locks, always follow this order
/// (outermost acquired first, innermost last):
///
///   Level 6 (outermost): PIPES (waking a waiter takes PROCESS_TABLE)
///   Level 5:             ACCOUNT_TABLE
///   Level 4:             PROCESS_TABLE
///   Level 3:             MOUNT_TABLE, PROCFS_LOCK
///   Level 2:             VFS_NODE_LOCK, OPEN_FILE_TABLE
///   Level 1:             TTY buffers, EXT2_WRITE_LOCK
///   Level 0:             BUFFER_CACHE
///   Leaf (innermost):    RUN_QUEUE (taken by set_state(); takes no other lock),
///                        fs::grace's GRACE (takes no other lock while held)
///
/// RULES:
/// - Never acquire a higher-level lock while holding a lower-level one.
/// - Never sleep or block while holding ANY spinlock.
/// - Never call close_open_file() with PROCESS_TABLE held: closing a pipe end
///   takes PIPES.
/// - Never drop a NodePin, or call resolve_path() or close_open_file(), under
///   a spinlock: finishing a delayed free takes PROCESS_TABLE.
/// - The buffer cache intentionally drops its lock before disk I/O.
/// - EXT2_WRITE_LOCK is the exception: it is held across disk I/O, which is
///   polled. It needs a sleeping lock if IDE ever becomes interrupt-driven.
/// - Per-CPU data (ThreadInfo, idle stacks) doesn't need locks,
///   it's only accessed by the owning CPU.
mod spinlock;

pub(crate) use spinlock::{Spinlock, SpinlockGuard};
