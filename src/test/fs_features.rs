//! Boot probe for the pseudo-filesystems

use core::fmt::Write;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::error::KernelError;
use crate::fs::{self, VfsNodeType};
use crate::{pr_err, pr_info};

struct Probe {
    passed: u32,
    failed: u32,
}

impl Probe {
    fn check(&mut self, name: &str, ok: bool) {
        if ok {
            self.passed += 1;
        } else {
            self.failed += 1;
            pr_err!("fs_features_boot_probe: FAILED {}", name);
        }
    }
}

// Only its address is used: fs::seq keys snapshots by mount pointer
static FAKE_MOUNT: u8 = 0;
const SEQ_INODE: u32 = 0xFFFF_0000;
// Larger than the 4 KiB first attempt, so generation has to grow twice.
const SEQ_LEN: usize = 10_000;
const SEQ_CHUNK: usize = 777;

static SEQ_CALLS: AtomicU32 = AtomicU32::new(0);

fn fake_mount() -> *const fs::Mount {
    &FAKE_MOUNT as *const u8 as *const fs::Mount
}

/// Fills `buf` with a pattern that changes with every generation, so a
/// stale or mixed snapshot is detectable.
fn seq_generate(buf: &mut [u8], len: usize) -> usize {
    let generation = SEQ_CALLS.fetch_add(1, Ordering::SeqCst);
    let count = len.min(buf.len());
    for (i, byte) in buf[..count].iter_mut().enumerate() {
        *byte = (i as u32).wrapping_mul(31).wrapping_add(generation) as u8;
    }
    count
}

unsafe fn seq_read(inode: u32, len: usize, buffer: &mut [u8], offset: usize) -> usize {
    unsafe {
        fs::seq::read(fake_mount(), inode, buffer, offset as u32, &mut |buf| {
            Ok(seq_generate(buf, len))
        })
        .unwrap_or(usize::MAX)
    }
}

unsafe fn seq_read_all(probe: &mut Probe, label: &str) -> Option<u8> {
    unsafe {
        let mut chunk = [0u8; SEQ_CHUNK];
        let mut offset = 0;
        let mut first = None;
        let mut consistent = true;
        loop {
            let count = seq_read(SEQ_INODE, SEQ_LEN, &mut chunk, offset);
            if count == usize::MAX {
                probe.check(label, false);
                return None;
            }
            if count == 0 {
                break;
            }
            let base = *first.get_or_insert(chunk[0]);
            for (i, &byte) in chunk[..count].iter().enumerate() {
                let expected = ((offset + i) as u32)
                    .wrapping_mul(31)
                    .wrapping_add(base as u32) as u8;
                consistent &= byte == expected;
            }
            offset += count;
        }
        probe.check(label, consistent && offset == SEQ_LEN);
        first
    }
}

unsafe fn probe_seq(probe: &mut Probe) {
    unsafe {
        let calls_before = SEQ_CALLS.load(Ordering::SeqCst);
        let first = seq_read_all(probe, "seq chunked read consistent");
        let generations = SEQ_CALLS.load(Ordering::SeqCst) - calls_before;
        // 4 KiB and 8 KiB attempts fill up, 16 KiB holds 10000 bytes.
        probe.check("seq buffer grows to fit", generations == 3);

        let mut byte = [0u8; 1];
        let calls_before = SEQ_CALLS.load(Ordering::SeqCst);
        probe.check(
            "seq read past end",
            seq_read(SEQ_INODE, SEQ_LEN, &mut byte, SEQ_LEN) == 0,
        );
        probe.check(
            "seq offset read served from snapshot",
            SEQ_CALLS.load(Ordering::SeqCst) == calls_before,
        );

        let second = seq_read_all(probe, "seq reread consistent");
        probe.check(
            "seq offset 0 regenerates",
            first.is_some() && first != second,
        );

        // Eight other files fill every slot and push the snapshot out.
        for inode in 1..=8 {
            let _ = seq_read(SEQ_INODE + inode, 16, &mut byte, 0);
        }
        let calls_before = SEQ_CALLS.load(Ordering::SeqCst);
        let _ = seq_read(SEQ_INODE, SEQ_LEN, &mut byte, 1);
        probe.check(
            "seq evicted snapshot regenerated",
            SEQ_CALLS.load(Ordering::SeqCst) > calls_before,
        );

        let failed = fs::seq::read(fake_mount(), SEQ_INODE + 9, &mut byte, 0, &mut |_| {
            Err(KernelError::IO)
        });
        probe.check(
            "seq generator error propagated",
            failed == Err(KernelError::IO),
        );
    }
}

unsafe fn path_of(node: *mut fs::VfsNode, buf: &mut [u8]) -> Result<&str, KernelError> {
    unsafe {
        let len = fs::node_path(node, buf)?;
        core::str::from_utf8(&buf[..len]).map_err(|_| KernelError::Inval)
    }
}

unsafe fn probe_node_path(probe: &mut Probe) {
    unsafe {
        let mut buf = [0u8; 64];
        probe.check(
            "node_path root",
            path_of(fs::ROOT_NODE, &mut buf) == Ok("/"),
        );

        let Ok(version) = fs::resolve_path("/proc/version", fs::ROOT_NODE) else {
            probe.check("resolve /proc/version", false);
            return;
        };
        probe.check(
            "node_path /proc/version",
            path_of(version, &mut buf) == Ok("/proc/version"),
        );

        let mut small = [0u8; 8];
        probe.check(
            "node_path too long",
            fs::node_path(version, &mut small) == Err(KernelError::NameTooLong),
        );
        probe.check(
            "node_path root in empty buffer",
            fs::node_path(fs::ROOT_NODE, &mut []) == Err(KernelError::Range),
        );
    }
}

unsafe fn probe_symlink(probe: &mut Probe) {
    unsafe {
        let task = crate::sched::current();
        if task.is_null() {
            pr_info!("fs_features_boot_probe: no current task, symlink checks skipped");
            return;
        }
        let Ok(proc_dir) = fs::resolve_path("/proc", fs::ROOT_NODE) else {
            probe.check("resolve /proc", false);
            return;
        };

        let saved_cwd = (*task).cwd;
        if saved_cwd.is_null() {
            (*task).cwd = proc_dir;
        }
        let mut dir = [0u8; 16];
        let dir_len = {
            let mut writer = fs::procfs::BufferWriter::new(&mut dir);
            let _ = write!(writer, "/proc/{}", (*task).pid);
            writer.written()
        };
        let dir = core::str::from_utf8(&dir[..dir_len]).unwrap_or("");
        let lent_dir = fs::resolve_path(dir, fs::ROOT_NODE).is_err()
            && fs::procfs::process_created((*task).pid, 0, 0).is_ok();

        check_cwd_link(probe, task);

        if lent_dir {
            fs::procfs::process_reaped((*task).pid);
        }
        (*task).cwd = saved_cwd;
    }
}

unsafe fn check_cwd_link(probe: &mut Probe, task: *mut crate::sched::TaskStruct) {
    unsafe {
        let mut path = [0u8; 32];
        let path_len = {
            let mut writer = fs::procfs::BufferWriter::new(&mut path);
            let _ = write!(writer, "/proc/{}/cwd", (*task).pid);
            writer.written()
        };
        let path = core::str::from_utf8(&path[..path_len]).unwrap_or("");

        let Ok(link) = fs::resolve_path_nofollow(path, fs::ROOT_NODE) else {
            pr_info!(
                "fs_features_boot_probe: {} missing, symlink checks skipped",
                path
            );
            return;
        };
        probe.check(
            "nofollow returns the link",
            (*link).node_type == VfsNodeType::Symlink,
        );

        let mut target = [0u8; 256];
        let mut expected = [0u8; 256];
        let target = (*link)
            .read_link(&mut target)
            .ok()
            .and_then(|len| core::str::from_utf8(&target[..len]).ok());
        probe.check(
            "read_link gives cwd path",
            target.is_some() && target == path_of((*task).cwd, &mut expected).ok(),
        );

        probe.check(
            "resolve_path follows the link",
            fs::resolve_path(path, fs::ROOT_NODE) == Ok((*task).cwd),
        );

        let mut buf = [0u8; 16];
        probe.check(
            "read_link on a file rejected",
            fs::resolve_path("/proc/version", fs::ROOT_NODE)
                .is_ok_and(|node| (*node).read_link(&mut buf) == Err(KernelError::Inval)),
        );
    }
}

unsafe fn read_printk(node: *mut fs::VfsNode) -> Option<u8> {
    unsafe {
        let mut buf = [0u8; 8];
        let len = (*node).read(&mut buf, 0).ok()?;
        core::str::from_utf8(&buf[..len]).ok()?.trim().parse().ok()
    }
}

unsafe fn probe_printk(probe: &mut Probe) {
    unsafe {
        let Ok(node) = fs::resolve_path("/proc/sys/kernel/printk", fs::ROOT_NODE) else {
            probe.check("resolve /proc/sys/kernel/printk", false);
            return;
        };
        let level = || crate::log::LOG_LEVEL.load(Ordering::Relaxed);
        let original = level();
        probe.check(
            "printk read matches level",
            read_printk(node) == Some(original),
        );

        let target: u8 = if original == 7 { 6 } else { 7 };
        let mut value = [target + b'0', b'\n'];
        probe.check("printk write accepted", (*node).write(&value, 0) == Ok(2));
        probe.check("printk write applied", level() == target);
        probe.check("printk reread", read_printk(node) == Some(target));

        for bad in [&b"8"[..], b"abc", b"", b"-1"] {
            if (*node).write(bad, 0) != Err(KernelError::Inval) {
                probe.check("printk invalid value rejected", false);
            }
        }
        probe.check("printk invalid left level alone", level() == target);

        value[0] = original + b'0';
        let _ = (*node).write(&value, 0);
        probe.check("printk restored", level() == original);
    }
}

#[unsafe(link_section = ".init.text")]
unsafe fn create_race_node(name: &str) -> Option<*mut fs::VfsNode> {
    unsafe { fs::create_child_node(fs::ROOT_NODE, name, VfsNodeType::CharDevice, 0o600).ok() }
}

unsafe fn is_race_node(node: *mut fs::VfsNode) -> bool {
    unsafe { (*node).node_type == VfsNodeType::CharDevice }
}

/// The free list is last-in first-out, so allocating right after a free gets
/// that same node back
unsafe fn next_alloc_is(node: *mut fs::VfsNode) -> bool {
    unsafe {
        let Ok(next) = fs::alloc_vfs_node() else {
            return false;
        };
        fs::free_vfs_node(next);
        next == node
    }
}

/// `open` on one CPU racing `unlink` on another, played out in order: `open`
/// resolves the file, `unlink` removes it, something else allocates a node,
/// and only then does `open` store the node it resolved. Freeing at the unlink
/// would hand that node to the allocation, and `open` would bind another file.
unsafe fn probe_open_racing_unlink(probe: &mut Probe) {
    unsafe {
        let (Some(victim), Some(neighbour)) = (
            create_race_node("fs_race_victim"),
            create_race_node("fs_race_neighbour"),
        ) else {
            probe.check("race: nodes created", false);
            return;
        };

        // "CPU A": open() has resolved the path.
        let pin = fs::pin_nodes();
        let resolved = fs::resolve_path("/fs_race_victim", fs::ROOT_NODE);
        probe.check("race: path resolves to the node", resolved == Ok(victim));

        // "CPU B": unlink() takes it out of the tree.
        probe.check(
            "race: unlink succeeds",
            fs::detach_child(fs::ROOT_NODE, victim).is_ok(),
        );
        probe.check(
            "race: unlinked path is gone",
            fs::resolve_path("/fs_race_victim", fs::ROOT_NODE).is_err(),
        );
        probe.check(
            "race: second unlink is refused",
            fs::detach_child(fs::ROOT_NODE, victim).is_err(),
        );

        // A path walk standing on the node must still be able to move on from it.
        probe.check(
            "race: pinned walker keeps its link",
            (*victim).next_of_kin == neighbour,
        );

        // "CPU C": a new allocation must not get the node open() still holds.
        probe.check("race: node not reused while pinned", !next_alloc_is(victim));
        probe.check("race: node intact while pinned", is_race_node(victim));

        // "CPU A": open() stores the node, then finishes.
        let Ok(file) = fs::alloc_open_file(victim, 1) else {
            probe.check("race: open file allocated", false);
            drop(pin);
            return;
        };
        drop(pin);
        probe.check("race: node not reused while open", !next_alloc_is(victim));
        probe.check("race: node intact while open", is_race_node(victim));

        fs::close_open_file(file);
        probe.check("race: last close recycles the node", next_alloc_is(victim));

        let _ = fs::detach_child(fs::ROOT_NODE, neighbour);
    }
}

/// A task can be killed while preempted inside a syscall, and then never drops
/// its pins. Nothing could be freed after that unless the kill releases them.
unsafe fn probe_killed_with_pin(probe: &mut Probe) {
    unsafe {
        if crate::sched::current().is_null() {
            return; // pins are only tracked per task
        }
        let Some(victim) = create_race_node("fs_race_killed") else {
            probe.check("killed: node created", false);
            return;
        };

        core::mem::forget(fs::pin_nodes());
        probe.check(
            "killed: unlink succeeds",
            fs::detach_child(fs::ROOT_NODE, victim).is_ok(),
        );
        probe.check("killed: pin holds the node", !next_alloc_is(victim));

        let pins = core::mem::take(&mut (*crate::sched::current()).vfs_pins);
        fs::release_task_pins(pins);
        probe.check(
            "killed: releasing pins frees the node",
            next_alloc_is(victim),
        );
    }
}

/// Unlinking the last entry must not make a directory look unloaded. Its inode
/// is the ext2 root, so a reload would fill it with the root's entries.
unsafe fn probe_emptied_directory(probe: &mut Probe) {
    unsafe {
        const EXT2_ROOT_INODE: u32 = 2;

        let Ok(dir) = fs::alloc_vfs_node() else {
            probe.check("emptied: directory created", false);
            return;
        };
        (*dir).node_type = VfsNodeType::Directory;
        (*dir).inode = EXT2_ROOT_INODE;
        (*dir).master = (*fs::ROOT_NODE).master;

        let created = fs::create_child_node(dir, "fs_only_child", VfsNodeType::CharDevice, 0o600);
        let Ok(child) = created else {
            probe.check("emptied: child created", false);
            fs::free_vfs_node(dir);
            return;
        };
        probe.check(
            "emptied: unlink succeeds",
            fs::detach_child(dir, child).is_ok(),
        );

        let reload = (*dir).lazy_load_directory();
        probe.check(
            "emptied: directory is not re-read",
            reload.is_ok() && (*dir).children.is_null(),
        );

        let mut stale = core::mem::take(&mut (*dir).children);
        while !stale.is_null() {
            let next = (*stale).next_of_kin;
            fs::free_vfs_node(stale);
            stale = next;
        }
        fs::free_vfs_node(dir);
    }
}

/// Whether the on-disk directory `dir` has an entry `name`, read back into a
/// throwaway node instead of trusting the VFS tree.
unsafe fn on_disk(dir: *mut fs::VfsNode, name: &[u8]) -> Option<bool> {
    unsafe {
        let scratch = fs::alloc_vfs_node().ok()?;
        (*scratch).node_type = VfsNodeType::Directory;
        (*scratch).inode = (*dir).inode;
        (*scratch).master = (*dir).master;
        let loaded = (*scratch).lazy_load_directory();

        let mut found = false;
        let mut child = core::mem::take(&mut (*scratch).children);
        while !child.is_null() {
            let next = (*child).next_of_kin;
            found |= (*child).name.starts_with(name) && (*child).name[name.len()] == 0;
            fs::free_vfs_node(child);
            child = next;
        }
        fs::free_vfs_node(scratch);
        loaded.ok().map(|_| found)
    }
}

/// `rm` on a real ext2 file: the entry leaves the disk, the inode comes back,
/// and a file unlinked while open stays readable until its last close.
unsafe fn probe_rm_on_disk(probe: &mut Probe) {
    const NAME: &str = "fs_rm_probe";
    const OTHER: &str = "fs_rm_other";
    const DATA: &[u8] = b"still here after unlink";

    unsafe {
        let _pin = fs::pin_nodes();
        let Ok(tmp) = fs::resolve_path("/tmp", fs::ROOT_NODE) else {
            probe.check("rm: /tmp exists", false);
            return;
        };
        probe.check("rm: /tmp loads", (*tmp).lazy_load_directory().is_ok());
        let create = |name| fs::create_child_node(tmp, name, VfsNodeType::File, 0o644);

        let Ok(file) = create(NAME) else {
            probe.check("rm: file created", false);
            return;
        };
        let inode = (*file).inode;
        probe.check("rm: file written", (*file).write(DATA, 0) == Ok(DATA.len()));
        probe.check("rm: entry on disk", on_disk(tmp, NAME.as_bytes()) == Some(true));

        probe.check("rm: unlink succeeds", fs::unlink_child(tmp, NAME, file).is_ok());
        probe.check("rm: entry gone from disk", on_disk(tmp, NAME.as_bytes()) == Some(false));

        // The second time round, unlink happens while the file is open. The
        // pin held above delays freeing, so inode reuse is checked after it.
        let Ok(file) = create(NAME) else {
            probe.check("rm: file recreated", false);
            return;
        };
        probe.check("rm: rewritten", (*file).write(DATA, 0) == Ok(DATA.len()));
        let Ok(open) = fs::alloc_open_file(file, 1) else {
            probe.check("rm: open file allocated", false);
            return;
        };
        probe.check("rm: unlink while open", fs::unlink_child(tmp, NAME, file).is_ok());

        let mut back = [0u8; DATA.len()];
        let read = (*file).read(&mut back, 0);
        probe.check("rm: open file still reads", read == Ok(DATA.len()) && back == *DATA);

        let open_inode = (*file).inode;
        let other = create(OTHER);
        probe.check(
            "rm: open file keeps its inode",
            other.is_ok_and(|node| (*node).inode != open_inode),
        );
        fs::close_open_file(open);
        if let Ok(node) = other {
            probe.check("rm: other file removed", fs::unlink_child(tmp, OTHER, node).is_ok());
        }
        pr_info!("fs_features_boot_probe: rm probe used inodes {}, {}", inode, open_inode);
    }
}

/// With every pin gone, the inodes `probe_rm_on_disk` unlinked must be free:
/// ext2 hands out the lowest free inode, so a new file gets the first one back.
unsafe fn probe_rm_frees_inode(probe: &mut Probe, inode: u32) {
    unsafe {
        let Ok(tmp) = fs::resolve_path("/tmp", fs::ROOT_NODE) else {
            return;
        };
        let name = "fs_rm_again";
        let Ok(file) = fs::create_child_node(tmp, name, VfsNodeType::File, 0o644) else {
            probe.check("rm: file created after free", false);
            return;
        };
        probe.check("rm: inode is reused", (*file).inode == inode);
        let _pin = fs::pin_nodes();
        probe.check("rm: cleanup", fs::unlink_child(tmp, name, file).is_ok());
    }
}

unsafe fn probe_unlink_races(probe: &mut Probe) {
    unsafe {
        probe_open_racing_unlink(probe);
        probe_killed_with_pin(probe);
        probe_emptied_directory(probe);
    }
}

unsafe fn probe_rm(probe: &mut Probe) {
    unsafe {
        // The lowest free inode now is the one the first file will get.
        let Ok(tmp) = fs::resolve_path("/tmp", fs::ROOT_NODE) else {
            return;
        };
        let Ok(marker) = fs::create_child_node(tmp, "fs_rm_marker", VfsNodeType::File, 0o644) else {
            probe.check("rm: marker created", false);
            return;
        };
        let lowest = (*marker).inode;
        {
            let _pin = fs::pin_nodes();
            probe.check("rm: marker removed", fs::unlink_child(tmp, "fs_rm_marker", marker).is_ok());
        }
        probe_rm_on_disk(probe);
        probe_rm_frees_inode(probe, lowest);
    }
}

pub(crate) fn fs_features_boot_probe() {
    let mut probe = Probe { passed: 0, failed: 0 };
    unsafe {
        probe_seq(&mut probe);
        probe_node_path(&mut probe);
        probe_symlink(&mut probe);
        probe_printk(&mut probe);
        probe_unlink_races(&mut probe);
        probe_rm(&mut probe);
    }

    if probe.failed == 0 {
        pr_info!("fs_features_boot_probe: all {} checks passed", probe.passed);
    } else {
        pr_err!(
            "fs_features_boot_probe: {} of {} checks failed",
            probe.failed,
            probe.passed + probe.failed
        );
    }
}

crate::late_initcall!(fs_features_boot_probe);
