/// # Lock Ordering Discipline (SMP Deadlock Prevention)
///
/// When acquiring multiple locks, always follow this order
/// (outermost acquired first, innermost last):
///
///   Level 5 (outermost): ACCOUNT_TABLE
///   Level 4:             PROCESS_TABLE  
///   Level 3:             MOUNT_TABLE
///   Level 2:             VFS_NODE_LOCK, OPEN_FILE_TABLE
///   Level 1:             TTY buffers, PIPE table
///   Level 0 (innermost): BUFFER_CACHE
///
/// RULES:
/// - Never acquire a higher-level lock while holding a lower-level one.
/// - Never sleep or block while holding ANY spinlock.
/// - The buffer cache intentionally drops its lock before disk I/O.
/// - Per-CPU data (ThreadInfo, idle stacks) doesn't need locks —
///   it's only accessed by the owning CPU.

mod spinlock;

pub(crate) use spinlock::Spinlock;
