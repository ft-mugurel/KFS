//! Boot probe for bugs that have been fixed once, so they stay fixed.
//! 1. heap span coalescing
//! 2. run queue invariants
//! 3. CPU identity after SMP bring-up
//! 4. ThreadInfo availability from the first instruction
//! 5. the timer tick rate that sleeps and timestamps are converted with
//! 6. VFS nodes going back to the pool.

use crate::error::KernelError;
use crate::interrupts::timer;
use crate::smp::{self, MAX_CPUS};
use crate::{drivers, fs, ipc, paging, pr_err, pr_info, sched, syscalls, x86};

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
            pr_err!("regressions_boot_probe: FAILED {}", name);
        }
    }
}

/// BSP and every AP must run as the cpu_id their LAPIC was registered under.
/// Broken when the BSP isn't first in the MADT, or when a late AP reads
/// trampoline data meant for the next one
fn probe_cpu_identity(probe: &mut Probe) {
    let bsp_apic = unsafe { smp::lapic::this_cpu_apic_id() };
    probe.check("BSP runs as cpu 0", unsafe { sched::current_cpu() } == 0);
    probe.check(
        "cpu 0 is the BSP's APIC",
        smp::cpu::apic_id_of(0) == bsp_apic,
    );
    probe.check(
        "APs agree with their cpu_id",
        smp::cpu::identity_mismatches() == 0,
    );

    let registered = smp::cpu::cpu_count();
    probe.check(
        "every registered CPU online",
        smp::cpu::online_count() == registered,
    );
    probe.check(
        "AP check-ins match online CPUs",
        smp::trampoline::AP_BOOTED_COUNT.load(core::sync::atomic::Ordering::SeqCst) as usize + 1
            == smp::cpu::online_count(),
    );

    let mut unique = true;
    for a in 0..registered.min(MAX_CPUS) {
        for b in a + 1..registered.min(MAX_CPUS) {
            unique &= smp::cpu::apic_id_of(a) != smp::cpu::apic_id_of(b);
        }
    }
    probe.check("APIC ids unique", unique);
}

/// Waits on timer interrupts until the RTC second changes; returns the tick
/// count right after the change. None, if it doesn't change within ~4sec
fn next_rtc_second(from: u32) -> Option<(u32, usize)> {
    for _ in 0..400 {
        x86::hlt();
        let now = drivers::rtc::now_epoch_seconds();
        if now != from {
            return Some((now, timer::get_ticks()));
        }
    }
    None
}

/// Counts global ticks across one RTC second. Sleeps and log timestamps are
/// converted with ms_to_ticks()/tick_frequency_hz(), so both must match it.
fn probe_tick_rate(probe: &mut Probe) {
    let flags = x86::read_eflags();
    unsafe { x86::enable_interrupts() };
    let start = drivers::rtc::now_epoch_seconds();
    let measured = next_rtc_second(start)
        .and_then(|(edge, first)| next_rtc_second(edge).map(|(_, second)| second - first));
    unsafe { x86::restore_eflags(flags) };

    let Some(measured) = measured else {
        probe.check("timer ticks advance across RTC seconds", false);
        return;
    };
    // Emulated clocks drift; a wrong conversion factor is off by 10x, not 25%.
    let close = |value: u64| value * 4 >= measured as u64 * 3 && value * 4 <= measured as u64 * 5;
    pr_info!(
        "regressions_boot_probe: {} ticks per RTC second, tick_frequency_hz={}",
        measured,
        timer::tick_frequency_hz()
    );
    probe.check(
        "tick_frequency_hz matches measured rate",
        close(timer::tick_frequency_hz() as u64),
    );
    probe.check(
        "ms_to_ticks(1000) is one second",
        close(timer::ms_to_ticks(1000)),
    );
}

fn socket_open_close() -> bool {
    let Ok(index) = ipc::create_socket() else {
        return false;
    };
    let Ok(node) = (unsafe { fs::alloc_vfs_node() }) else {
        ipc::close_socket(index);
        return false;
    };
    unsafe {
        (*node).node_type = fs::VfsNodeType::Socket;
        (*node).inode = index as u32;
    }
    let Ok(file) = fs::alloc_open_file(node, 1) else {
        ipc::close_socket(index);
        unsafe { fs::free_vfs_node(node) };
        return false;
    };
    unsafe { fs::close_open_file(file) };
    true
}

/// The node must survive its unlink while open, and be freed by the close.
fn unlink_while_open(created: crate::error::KResult<*mut fs::VfsNode>) -> bool {
    let Ok(node) = created else {
        return false;
    };
    let Ok(file) = fs::alloc_open_file(node, 1) else {
        return false;
    };
    unsafe {
        let detached = fs::detach_child(fs::ROOT_NODE, node).is_ok();
        let survived = (*node).node_type == fs::VfsNodeType::CharDevice;
        let unlinked_twice = fs::detach_child(fs::ROOT_NODE, node).is_ok();
        fs::close_open_file(file);
        detached && survived && !unlinked_twice
    }
}

/// A pin keeps an unlinked node alive, and a pinned open that lands after the
/// unlink keeps it until close, as with `open` racing `unlink`.
fn unlink_while_pinned(created: crate::error::KResult<*mut fs::VfsNode>) -> bool {
    let Ok(node) = created else {
        return false;
    };
    let pin = fs::pin_nodes();
    unsafe {
        let detached = fs::detach_child(fs::ROOT_NODE, node).is_ok();
        let pinned = (*node).node_type == fs::VfsNodeType::CharDevice;
        let Ok(file) = fs::alloc_open_file(node, 1) else {
            return false;
        };
        drop(pin);
        let open = (*node).node_type == fs::VfsNodeType::CharDevice;
        fs::close_open_file(file);
        detached && pinned && open
    }
}

/// The node pool is fixed, so each of these runs past its size: a single
/// leaked node per round exhausts it and the round starts failing.
fn probe_vfs_node_reuse(probe: &mut Probe) {
    let rounds = fs::MAX_VFS_NODES;
    let root = sched::Credentials::root();

    let pipes = (0..rounds).all(|_| {
        syscalls::create_pipe_files(&root).is_ok_and(|(read, write)| {
            unsafe {
                fs::close_open_file(read);
                fs::close_open_file(write);
            }
            true
        })
    });
    probe.check("pipe nodes return to the pool on close", pipes);
    probe.check(
        "socket nodes return to the pool on close",
        (0..rounds).all(|_| socket_open_close()),
    );

    let failed_create = (0..rounds).all(|_| {
        let created = unsafe {
            fs::create_child_node(fs::ROOT_NODE, "vfs_probe", fs::VfsNodeType::Fifo, 0o644)
        };
        matches!(created, Err(KernelError::OpNotSupp))
    });
    probe.check("failed node creation returns the node", failed_create);

    let create = || unsafe {
        fs::create_child_node(
            fs::ROOT_NODE,
            "vfs_probe",
            fs::VfsNodeType::CharDevice,
            0o600,
        )
    };
    let unlinked = (0..rounds).all(|_| {
        create().is_ok_and(|node| unsafe { fs::detach_child(fs::ROOT_NODE, node) }.is_ok())
    });
    probe.check("unlinked nodes return to the pool", unlinked);

    let unlinked_open = (0..rounds).all(|_| unlink_while_open(create()));
    probe.check("unlinked open nodes last until close", unlinked_open);

    let unlinked_pinned = (0..rounds).all(|_| unlink_while_pinned(create()));
    probe.check("pinned nodes survive unlink", unlinked_pinned);
}

#[unsafe(link_section = ".init.text")]
pub(crate) fn regressions_boot_probe() {
    let mut probe = Probe { passed: 0, failed: 0 };

    paging::kernel_heap_span_selftest(&mut |name, ok| probe.check(name, ok));
    sched::runqueue_selftest(&mut |name, ok| probe.check(name, ok));
    probe_cpu_identity(&mut probe);
    probe.check(
        "no ThreadInfo canary misses",
        sched::canary_error_count() == 0,
    );
    probe_tick_rate(&mut probe);
    probe_vfs_node_reuse(&mut probe);

    if probe.failed == 0 {
        pr_info!("regressions_boot_probe: all {} checks passed", probe.passed);
    } else {
        pr_err!(
            "regressions_boot_probe: {} of {} checks failed",
            probe.failed,
            probe.passed + probe.failed
        );
    }
}

crate::late_initcall!(regressions_boot_probe);
